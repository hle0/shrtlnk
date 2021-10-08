use std::{collections::BTreeMap, fs::File, io::Read, sync::Arc};

use anyhow::{anyhow, Result};
use serde::Deserialize;
use tide::{Redirect, Request};
use tokio::sync::RwLock;

#[derive(Deserialize, Default)]
pub struct LinkSettings {
    dst: String,
}

#[derive(Deserialize, Default)]
pub struct Config {
    host: String,
    port: u16,
    links: BTreeMap<String, LinkSettings>,
}

impl Config {
    pub fn check(&self) -> Result<()> {
        for x in self.links.keys() {
            if !x.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || (c == '_')
                    || (c == '-')
                    || (c == '.')
                    || (c == '$')
                    || (c == '#')
                    || (c == '~')
                    || (c == '@')
            }) {
                return Err(anyhow!(
                    "Configured shortlink '{:?}' contains invalid characters",
                    x
                ));
            }
        }

        Ok(())
    }

    pub fn changes_require_restart(&self, other: &Self) -> bool {
        (self.host != other.host) || (self.port != other.port)
    }
}

pub struct Application {
    config_location: String,
    config: RwLock<Config>,
}

impl Application {
    pub async fn new(config_location: String) -> Result<Self> {
        let me = Self {
            config_location,
            config: RwLock::new(Config::default()),
        };

        me.reload_config(false).await?;

        Ok(me)
    }

    #[cfg(unix)]
    async fn signal_monitor(me: Arc<Self>) {
        use tokio::signal::unix::{signal, SignalKind};

        let mut s =
            signal(SignalKind::hangup()).expect("Failed to create signal handler for SIGHUP");

        loop {
            s.recv().await;
            match me.reload_config(true).await {
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

    pub async fn reload_config(&self, check: bool) -> Result<()> {
        let mut content = String::new();
        File::open(self.config_location.as_str())?.read_to_string(&mut content)?;

        let new_config: Config = toml::from_str(content.as_str())?;
        new_config.check()?;

        {
            let mut old_config = self.config.write().await;

            if check && new_config.changes_require_restart(&*old_config) {
                return Err(anyhow!(
                    "These configuration changes would require a restart."
                ));
            }

            *old_config = new_config;
        }

        Ok(())
    }

    pub async fn setup_server(me: Arc<Self>) -> Result<()> {
        let mut server = tide::Server::new();
        server
            .at("/")
            .get(|_| async { Ok(Redirect::permanent("/_")) });
        server
            .at("/_")
            .get(|_| async { Ok(Redirect::permanent("/_/_")) });

        let me_clone: Arc<Application> = me.clone();
        server.at("/_/*path").get(move |r: Request<()>| {
            let me_clone = me_clone.clone();
            async move { me_clone.handle_special_request(&r).await }
        });

        let me_clone: Arc<Application> = me.clone();
        server.at("/:short").get(move |r: Request<()>| {
            let me_clone = me_clone.clone();
            async move { me_clone.handle_normal_request(&r).await }
        });

        let listener = {
            let config = me.config.read().await;
            format!("{}:{}", config.host, config.port)
        };

        server.listen(listener).await?;

        Ok(())
    }

    pub async fn handle_normal_request(&self, req: &Request<()>) -> tide::Result {
        let config = self.config.read().await;
        return match config.links.get(req.param("short")?) {
            Some(link) => Ok(Redirect::temporary(link.dst.as_str()).into()),
            None => Err(tide::Error::new(
                404,
                anyhow!("The requested shortlink does not exist."),
            )),
        };
    }

    pub async fn handle_special_request(&self, _req: &Request<()>) -> tide::Result {
        Err(tide::Error::new(404, anyhow!("Not found.")))
    }
}
