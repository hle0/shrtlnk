use std::{convert::Infallible, fs::File, io::Read, sync::Arc};

use crate::config::{CheckConfig, Config};
use anyhow::{anyhow, Result};
use hyper::{
    body::{Body, Bytes},
    server::conn::AddrStream,
    service::{make_service_fn, service_fn},
    Request, Response, Server,
};
use tokio::sync::RwLock;

pub struct Application {
    config_location: String,
    config: RwLock<Option<Config>>,
}

impl Application {
    pub async fn new(config_location: String) -> Result<Self> {
        let me = Self {
            config_location,
            config: RwLock::new(None),
        };

        if !me.config_location.is_empty() {
            me.reload_config().await?;
        }

        Ok(me)
    }

    #[allow(dead_code)]
    pub async fn from_config(config: Config) -> Result<Self> {
        let me = Self::new("".to_string()).await?;
        me.try_load_config(config).await?;
        Ok(me)
    }

    #[cfg(unix)]
    async fn signal_monitor(me: Arc<Self>) {
        use tokio::signal::unix::{signal, SignalKind};

        let mut s =
            signal(SignalKind::hangup()).expect("Failed to create signal handler for SIGHUP");

        loop {
            s.recv().await;
            match me.reload_config().await {
                Ok(_) => eprintln!("Successfully reloaded configuration"),
                Err(e) => eprintln!("Got an error during configuration reload: {}", e),
            };
        }
    }

    pub async fn spawn(me: Arc<Self>) -> Result<()> {
        #[cfg(unix)]
        {
            use tokio::task;
            task::spawn(Self::signal_monitor(me.clone()));
        }

        Self::setup_server(me.clone()).await?;

        Ok(())
    }

    pub async fn reload_config(&self) -> Result<()> {
        let mut content = String::new();
        File::open(self.config_location.as_str())?.read_to_string(&mut content)?;

        let new_config: Config = toml::from_str(content.as_str())?;

        self.try_load_config(new_config).await
    }

    pub async fn try_load_config(&self, mut new_config: Config) -> Result<()> {
        new_config.check()?;

        let mut old_config = self.config.write().await;

        if let Some(ref old_config_struct) = *old_config {
            if new_config.requires_restart(old_config_struct) {
                return Err(anyhow!(
                    "These configuration changes would require a restart."
                ));
            }
        }

        *old_config = Some(new_config);

        Ok(())
    }

    pub async fn setup_server(me: Arc<Self>) -> Result<()> {
        let the_service = make_service_fn(|_: &AddrStream| {
            let me_clone = me.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |r| {
                    let me_clone = me_clone.clone();
                    async move { me_clone.handle_request(r).await }
                }))
            }
        });

        let listener = {
            if let Some(ref some_config) = *me.config.read().await {
                some_config.host.spec()
            } else {
                return Err(anyhow!(
                    "server was not configured before attempting to run."
                ));
            }
        };

        if let Err(e) = Server::bind(&listener).serve(the_service).await {
            return Err(anyhow!(e));
        }

        Ok(())
    }

    pub async fn handle_request(&self, req: Request<Body>) -> anyhow::Result<Response<Body>> {
        let config = self.config.read().await;
        if let Some(ref config_struct) = *config {
            if let Some(handler) = config_struct
                .handlers
                .iter()
                .find(|m| m.matcher.matches(&req))
            {
                handler.page.serve(req).await
            } else {
                Ok(Response::builder()
                    .status(404)
                    .body(Body::from(Bytes::from_static(b"404: not found")))?)
            }
        } else {
            panic!("The server attempted to serve a page before it was configured.");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::config::Config;

    use super::Application;

    async fn working_dummy_task() -> (
        Arc<Application>,
        tokio::task::JoinHandle<anyhow::Result<()>>,
    ) {
        let app = Application::from_config(Config::working_dummy_config())
            .await
            .unwrap();
        let arc = Arc::new(app);
        let task = tokio::task::spawn(Application::setup_server(arc.clone()));
        (arc.clone(), task)
    }

    async fn wait_until_loaded(url: String) {
        for wait in 1..100 {
            if let Ok(_) = reqwest::get(url.clone()).await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        }

        panic!("server never finished starting!");
    }

    #[tokio::test]
    async fn main_test() {
        let _ = working_dummy_task().await;
        let url_base = format!("http://{}", Config::working_dummy_hostspec().spec());
        wait_until_loaded(format!("{}/abc", url_base)).await;
        assert_eq!(
            reqwest::get(url_base.clone() + "/abc")
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "abc"
        );
        assert_eq!(
            reqwest::get(url_base.clone() + "/redir")
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "abc"
        );
    }
}
