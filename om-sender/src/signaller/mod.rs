// TODO: look into TLS

pub mod handlers;
pub mod protocol;
pub mod server;

pub async fn run_server(prod_peer_tx: tokio::sync::oneshot::Sender<String>) {
    let server = server::Server::spawn(handlers::Handler::new, prod_peer_tx);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8443")
        .await
        .unwrap();

    while let Ok((stream, address)) = listener.accept().await {
        let mut server_clone = server.clone();
        log::info!("Accepting connection from {address}");
        tokio::task::spawn(async move { server_clone.accept_async(stream).await });
    }
}
