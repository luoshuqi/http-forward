use std::process::exit;

use log::error;

use http_forward::client;

#[tokio::main]
async fn main() {
    if let Err(e) = client::run().await {
        error!("{}", e);
        exit(1);
    }
}
