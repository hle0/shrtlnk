use std::{borrow::Borrow, collections::BTreeMap, fs::File, io::Read};

use anyhow::anyhow;
use serde::Deserialize;

pub trait CheckConfig {
    fn check(&mut self) -> anyhow::Result<()>;
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum StaticPage {
    #[serde(rename = "redirect")]
    Redirect { to: String },
    #[serde(rename = "string")]
    Embedded {
        data: String,
        #[serde(default = "StaticPage::default_content_type")]
        content_type: String,
    },
    #[serde(rename = "file")]
    StaticFile {
        path: String,
        #[serde(default = "StaticPage::default_content_type")]
        content_type: String,
        #[serde(skip)]
        cached_data: String,
    },
}

impl StaticPage {
    fn default_content_type() -> String {
        "text/html".to_string()
    }

    pub fn serve(&self) -> tide::Result {
        match &self {
            Self::Redirect { to } => Ok(tide::Redirect::temporary(to).into()),
            Self::Embedded { data, content_type } => Ok(tide::Response::builder(200)
                .header("content-type", content_type)
                .body(data.clone())
                .build()),
            Self::StaticFile {
                content_type,
                cached_data,
                ..
            } => Ok(tide::Response::builder(200)
                .header("content-type", content_type)
                .body(cached_data.clone())
                .build()),
        }
    }
}

impl CheckConfig for StaticPage {
    fn check(&mut self) -> anyhow::Result<()> {
        if let StaticPage::StaticFile {
            path, cached_data, ..
        } = self
        {
            File::open(path)?.read_to_string(cached_data)?;
        };
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
        "localhost".to_string()
    }

    fn default_port() -> u16 {
        8387
    }

    pub fn spec_string(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Deserialize)]
pub struct ErrorPages {
    #[serde(default = "ErrorPages::default_page_not_found")]
    pub not_found: StaticPage,
    #[serde(default = "ErrorPages::default_page_no_path")]
    pub no_path: StaticPage,
}

impl CheckConfig for ErrorPages {
    fn check(&mut self) -> anyhow::Result<()> {
        self.not_found.check()?;
        self.no_path.check()?;

        Ok(())
    }
}

impl Default for ErrorPages {
    fn default() -> Self {
        Self {
            not_found: Self::default_page_not_found(),
            no_path: Self::default_page_no_path(),
        }
    }
}

impl ErrorPages {
    fn default_page_no_path() -> StaticPage {
        StaticPage::Redirect {
            to: "/_".to_string(),
        }
    }

    fn default_page_not_found() -> StaticPage {
        StaticPage::Embedded {
            data: "404: not found.".to_string(),
            content_type: "text/html".to_string(),
        }
    }
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(flatten, default)]
    pub host: HostSpec,
    pub pages: BTreeMap<String, StaticPage>,
    #[serde(default)]
    pub errors: ErrorPages,
}

impl CheckConfig for Config {
    fn check(&mut self) -> anyhow::Result<()> {
        self.host.check()?;
        self.errors.check()?;
        for (key, value) in self.pages.iter_mut() {
            if key.is_empty() {
                return Err(anyhow!(
                    "one of the pages has an empty prefix. this means it won't be accessible. \
                     you probably want to change the index page via errors.no_path instead."
                ));
            }

            if key.starts_with('/') {
                return Err(anyhow!(
                    "the page key '{:?}' starts with a slash ('/'). this means it won't be routed properly. \
                     if you're trying to map an absolute prefix, just remove the leading slash.",
                    key
                ));
            }

            if key.ends_with('/') {
                return Err(anyhow!(
                    "the page key '{:?}' ends with a slash ('/'). this is invalid. \
                     but, any trailing slashes in a path are automatically discarded during routing. \
                     you should just remove the trailing slashes from the key.",
                    key
                ));
            }

            if key.contains("//") {
                return Err(anyhow!(
                    "the page key '{:?}' contains at least two consecutive slashes ('//'). \
                     this means the key will not be routed properly. \
                     you should make sure all keys don't start or end with slashes, \
                     and never have more than one consecutive slash.",
                    key
                ));
            }

            value.check()?;
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
            host: "localhost".to_string(),
            port: 43982, // a random port unlikely to be taken
        }
    }

    #[cfg(test)]
    pub fn working_dummy_config() -> Self {
        Self {
            host: Self::working_dummy_hostspec(),
            pages: {
                let mut map = BTreeMap::new();
                map.insert(
                    "abc".to_string(),
                    StaticPage::Embedded {
                        data: "abc".to_string(),
                        content_type: "text/plain".to_string(),
                    },
                );
                map.insert(
                    "redir".to_string(),
                    StaticPage::Redirect {
                        to: "/abc".to_string(),
                    },
                );
                map
            },
            errors: ErrorPages::default(),
        }
    }
}
