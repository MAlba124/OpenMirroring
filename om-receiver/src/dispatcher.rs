use std::sync::{Arc, Mutex};

use log::info;
use tokio::net::{TcpListener, ToSocketAddrs};

use crate::{session::SessionId, Event};

pub struct Dispatcher {
    listener: TcpListener,
    event_tx: tokio::sync::mpsc::Sender<Event>,
}

impl Dispatcher {
    pub async fn new<A>(addr: A, event_tx: tokio::sync::mpsc::Sender<Event>) -> tokio::io::Result<Self>
    where
        A: ToSocketAddrs,
    {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { listener, event_tx })
    }

    // TODO: Web socket listener
    pub async fn run(self) -> tokio::io::Result<()> {
        info!("Listening on {:?}", self.listener.local_addr());

        let mut id: SessionId = 0;

        loop {
            let (net_stream, _) = self.listener.accept().await?;
            self.event_tx
                .send(Event::CreateSessionRequest {
                    net_stream_mutex: Arc::new(Mutex::new(Some(net_stream))),
                    id,
                })
                .await
                .unwrap();
            id += 1;
        }
    }
}
