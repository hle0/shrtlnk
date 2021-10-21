use std::{borrow::Borrow, fs::File, io::Read, net::SocketAddr};

use anyhow::{anyhow, Context};
use hyper::{
    body::{Body, Bytes},
    Request, Response, Uri,
};
use serde::Deserialize;

pub trait CheckConfig {
    fn check(&mut self) -> anyhow::Result<()>;
}

#[derive(Deserialize)]
#[serde(tag = "matches")]
pub enum Matcher {
    #[serde(rename = "path")]
    Path { path: String },
    #[serde(rename = "regex")]
    Regex {
        expr: String,
        #[serde(skip)]
        compiled: Option<regex::Regex>,
    },
    #[serde(rename = "any")]
    Any { of: Vec<Matcher> },
    #[serde(rename = "all")]
    All { of: Vec<Matcher> },
    #[serde(rename = "not")]
    Not { matcher: Box<Matcher> },
    #[serde(rename = "root")]
    Root,
}

impl Matcher {
    pub fn matches(&self, req: &Request<Body>) -> bool {
        match self {
            Self::Path { path } => {
                let req_path = req.uri().path();

                req_path.trim_start_matches('/').trim_end_matches('/')
                    == path.as_str().trim_start_matches('/').trim_end_matches('/')
            }
            Self::Regex { compiled, .. } => compiled.as_ref().unwrap().is_match(req.uri().path()),
            Self::Any { of } => {
                for matcher in of {
                    if matcher.matches(req) {
                        return true;
                    }
                }

                false
            }
            Self::All { of } => {
                for matcher in of {
                    if !matcher.matches(req) {
                        return false;
                    }
                }

                true
            }
            Self::Not { matcher } => !matcher.matches(req),
            Self::Root => req.uri().path().chars().all(|c| c == '/'),
        }
    }
}

impl CheckConfig for Matcher {
    fn check(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Path { .. } => Ok(()),
            Self::Regex { expr, compiled } => {
                *compiled = Some(regex::Regex::new(expr.as_str())?);

                Ok(())
            }
            Self::All { of } => {
                if of.is_empty() {
                    return Err(anyhow!("no submatchers. there should be at least one.")
                        .context("inside a MatchesAll matcher block"));
                }

                for matcher in of {
                    if let Err(e) = matcher.check() {
                        return Err(e.context("inside a MatchesAll matcher block"));
                    }
                }

                Ok(())
            }
            Self::Any { of } => {
                if of.is_empty() {
                    return Err(anyhow!("no submatchers. there should be at least one.")
                        .context("inside a MatchesAny matcher block"));
                }

                for matcher in of {
                    if let Err(e) = matcher.check() {
                        return Err(e.context("inside a MatchesAny matcher block"));
                    }
                }

                Ok(())
            }
            Self::Not { matcher } => matcher.check(),
            Self::Root => Ok(()),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum StaticPage {
    #[serde(rename = "redirect")]
    Redirect { to: String },
    #[serde(rename = "string")]
    Embedded {
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
        #[serde(default = "StaticPage::default_content_type")]
        content_type: String,
    },
    #[serde(rename = "file")]
    StaticFile {
        path: String,
        #[serde(default = "StaticPage::default_content_type")]
        content_type: String,
        #[serde(skip)]
        cached_data: Vec<u8>,
    },
    #[serde(rename = "proxy")]
    ReverseProxy {
        #[serde(default = "StaticPage::default_scheme")]
        scheme: String,
        host: String,
        #[serde(skip)]
        client: hyper::Client<hyper::client::HttpConnector>,
    },
}

impl StaticPage {
    fn default_scheme() -> String {
        "http".to_string()
    }

    fn default_content_type() -> String {
        "text/html".to_string()
    }

    pub async fn serve(&self, req: Request<Body>) -> anyhow::Result<Response<Body>> {
        match &self {
            Self::Redirect { to } => Ok(Response::builder()
                .status(307)
                .header("Location", to)
                .body(Body::empty())?),
            Self::Embedded { data, content_type } => Ok(Response::builder()
                .status(200)
                .header("Content-Type", content_type)
                .body(Bytes::copy_from_slice(data.as_slice()).into())?),
            Self::StaticFile {
                content_type,
                cached_data,
                ..
            } => Ok(Response::builder()
                .status(200)
                .header("Content-Type", content_type)
                .body(Bytes::copy_from_slice(cached_data.as_slice()).into())?),
            Self::ReverseProxy {
                scheme,
                host,
                client,
            } => {
                let mut parts = req.uri().clone().into_parts();
                parts.scheme = Some(scheme.parse()?);
                parts.authority = Some(host.parse()?);

                let uri = Uri::from_parts(parts)?;
                /* fold the old headers into the new request */
                let new_req = req
                    .headers()
                    .into_iter()
                    .fold(
                        Request::builder().method(req.method()).uri(uri),
                        |builder, (name, value)| builder.header(name, value),
                    )
                    .body(req.into_body())?;

                Ok(client.request(new_req).await?)
            }
        }
    }
}

impl CheckConfig for StaticPage {
    fn check(&mut self) -> anyhow::Result<()> {
        if let StaticPage::StaticFile {
            path, cached_data, ..
        } = self
        {
            if let Err(e) = File::open(path).and_then(|mut x| x.read_to_end(cached_data)) {
                return Err(anyhow!(e).context("inside a StaticFile page"));
            }
        };
        Ok(())
    }
}

#[derive(Deserialize)]
pub struct Handler {
    #[serde(rename = "must_match")]
    pub matcher: Matcher,
    #[serde(flatten)]
    pub page: StaticPage,
}

impl CheckConfig for Handler {
    fn check(&mut self) -> anyhow::Result<()> {
        self.matcher.check().context("inside the root matcher")?;
        self.page.check().context("inside the page")?;
        Ok(())
    }
}

#[derive(Deserialize, PartialEq, Eq)]
pub struct HostSpec {
    #[serde(default = "HostSpec::default_host")]
    pub host: String,
    #[serde(default = "HostSpec::default_port")]
    pub port: u16,
}

impl CheckConfig for HostSpec {
    fn check(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

impl Default for HostSpec {
    fn default() -> Self {
        Self {
            host: Self::default_host(),
            port: Self::default_port(),
        }
    }
}

impl HostSpec {
    fn default_host() -> String {
        "127.0.0.1".to_string()
    }

    fn default_port() -> u16 {
        8387
    }

    pub fn spec(&self) -> SocketAddr {
        SocketAddr::new(self.host.parse().unwrap(), self.port)
    }
}

#[derive(Deserialize)]
pub struct ErrorPages {
    #[serde(default = "ErrorPages::default_page_not_found")]
    pub not_found: StaticPage,
}

impl CheckConfig for ErrorPages {
    fn check(&mut self) -> anyhow::Result<()> {
        self.not_found
            .check()
            .context("inside the not_found ErrorPage")?;

        Ok(())
    }
}

impl Default for ErrorPages {
    fn default() -> Self {
        Self {
            not_found: Self::default_page_not_found(),
        }
    }
}

impl ErrorPages {
    fn default_page_not_found() -> StaticPage {
        StaticPage::Embedded {
            data: "404: not found.".as_bytes().to_vec(),
            content_type: "text/html".to_string(),
        }
    }
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(flatten, default)]
    pub host: HostSpec,
    pub handlers: Vec<Handler>,
    #[serde(default)]
    pub errors: ErrorPages,
}

impl CheckConfig for Config {
    fn check(&mut self) -> anyhow::Result<()> {
        self.host.check().context("inside the HostSpec")?;
        self.errors.check().context("inside the error handlers")?;
        for (i, handler) in self.handlers.iter_mut().enumerate() {
            handler
                .check()
                .context(format!("inside handler {} (counting from 0)", i))?;
        }

        Ok(())
    }
}

impl Config {
    pub fn requires_restart<T: Borrow<Self>>(&self, other: T) -> bool {
        self.host != other.borrow().host
    }

    #[cfg(test)]
    pub fn working_dummy_hostspec() -> HostSpec {
        HostSpec {
            host: "127.0.0.1".to_string(),
            port: 43982, // a random port unlikely to be taken
        }
    }

    #[cfg(test)]
    pub fn working_dummy_config() -> Self {
        Self {
            host: Self::working_dummy_hostspec(),
            handlers: vec![
                Handler {
                    matcher: Matcher::Path {
                        path: "abc".to_string(),
                    },
                    page: StaticPage::Embedded {
                        data: "abc".as_bytes().to_vec(),
                        content_type: "text/plain".to_string(),
                    },
                },
                Handler {
                    matcher: Matcher::Path {
                        path: "redir".to_string(),
                    },
                    page: StaticPage::Redirect {
                        to: "/abc".to_string(),
                    },
                },
            ],
            errors: ErrorPages::default(),
        }
    }
}
