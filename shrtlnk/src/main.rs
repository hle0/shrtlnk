use std::sync::Arc;

use tokio;

mod app;

#[tokio::main]
async fn main() {
    let my_app = app::Application::new(String::from("./config.toml"))
        .await
        .expect("Could not initialize application");
    app::Application::spawn(Arc::new(my_app)).await.unwrap();
}
