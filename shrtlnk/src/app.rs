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
        let url_base = format!("http://{}", Config::working_dummy_hostspec().spec_string());
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
