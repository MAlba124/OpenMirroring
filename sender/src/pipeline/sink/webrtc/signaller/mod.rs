use tokio::sync::mpsc::Sender;

pub mod handlers;
pub mod protocol;
pub mod server;

pub async fn run_server(prod_peer_tx: Sender<crate::Event>) {
    let server = server::Server::spawn(handlers::Handler::new, prod_peer_tx);
    // TODO: use random port
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8443").await.unwrap();

    while let Ok((stream, address)) = listener.accept().await {
        let mut server_clone = server.clone();
        log::info!("Accepting connection from {address}");
        tokio::task::spawn(async move { server_clone.accept_async(stream).await });
    }
}
