use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

pub mod handlers;
pub mod protocol;
pub mod server;

pub async fn run_server(
    peer_id: Arc<Mutex<Option<String>>>,
    mut quit_signal: oneshot::Receiver<()>,
) {
    let server = server::Server::spawn(handlers::Handler::new, peer_id);

    // TODO: use random port
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8443").await.unwrap();

    log::debug!("WebRTC signaller running");

    loop {
        tokio::select! {
            _ = &mut quit_signal => {
                break;
            }
            res = listener.accept() => {
                let Ok((stream, address)) = res else {
                    break;
                };
                let mut server_clone = server.clone();
                log::info!("Accepting connection from {address}");
                tokio::task::spawn(async move { server_clone.accept_async(stream).await });
            }
        }
    }
}
