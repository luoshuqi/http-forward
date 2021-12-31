use std::process::exit;

use log::error;

use http_forward::server;

#[tokio::main]
async fn main() {
    if let Err(e) = server::run().await {
        error!("{}", e);
        exit(1);
    }
}
