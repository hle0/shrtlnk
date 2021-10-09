use std::{fs::File, io::Read, sync::Arc};

use anyhow::{anyhow, Result};
use tide::Request;
use tokio::sync::RwLock;

use crate::config::{CheckConfig, Config};

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

        me.reload_config().await?;

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

        let mut new_config: Config = toml::from_str(content.as_str())?;
        new_config.check()?;

        {
            let mut old_config = self.config.write().await;

            if let Some(ref old_config_struct) = *old_config {
                if new_config.requires_restart(old_config_struct) {
                    return Err(anyhow!(
                        "These configuration changes would require a restart."
                    ));
                }
            }

            *old_config = Some(new_config);
        }

        Ok(())
    }

    pub async fn setup_server(me: Arc<Self>) -> Result<()> {
        let mut server = tide::Server::new();

        let me_clone: Arc<Application> = me.clone();
        let handler = move |r: Request<()>| {
            let me_clone = me_clone.clone();
            async move { me_clone.handle_request(&r).await }
        };

        server.at("/").get(handler.clone());
        server.at("/*path").get(handler.clone());

        let listener = {
            if let Some(ref some_config) = *me.config.read().await {
                some_config.host.spec_string()
            } else {
                return Err(anyhow!(
                    "server was not configured before attempting to run."
                ));
            }
        };

        server.listen(listener).await?;

        Ok(())
    }

    pub async fn handle_request(&self, req: &Request<()>) -> tide::Result {
        let config = self.config.read().await;
        if let Some(ref config_struct) = *config {
            if let Ok(path) = req.param("path") {
                let real_path = path.trim_end_matches('/');
                if let Some(page) = config_struct.pages.get(real_path) {
                    page.serve()
                } else {
                    config_struct.errors.not_found.serve()
                }
            } else {
                config_struct.errors.no_path.serve()
            }
        } else {
            Err(anyhow!("The server attempted to serve a page before it was configured.").into())
        }
    }
}
