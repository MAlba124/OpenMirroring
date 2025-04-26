use anyhow::Result;
use log::info;
use simple_mdns::async_discovery::ServiceDiscovery;
use tokio::net::TcpListener;

use crate::{session::SessionId, Event};

pub struct Dispatcher {
    listener: TcpListener,
    event_tx: tokio::sync::mpsc::Sender<Event>,
    discovery: ServiceDiscovery,
}

impl Dispatcher {
    pub async fn new(event_tx: tokio::sync::mpsc::Sender<Event>) -> Result<Self> {
        let listener = TcpListener::bind("0.0.0.0:46899").await?;

        let instance_info = {
            let mut i = simple_mdns::InstanceInformation::new(format!(
                "OpenMirroring-{}",
                gethostname::gethostname().to_string_lossy()
            ));

            for addr in common::net::get_all_ip_addresses()
                .into_iter()
                .filter(|a| a.is_ipv4() && !a.is_loopback())
            {
                i = i.with_ip_address(addr).with_port(46899);
                log::debug!("Added {addr:?} as service");
            }

            i.with_port(46899)
        };

        let mut discovery = ServiceDiscovery::new(instance_info, "_fcast._tcp.local", 60).unwrap();
        discovery.announce(false).await.unwrap();

        Ok(Self {
            listener,
            event_tx,
            discovery,
        })
    }

    // TODO: Websocket listener
    // TODO: Fin oneshot
    pub async fn run(self) -> tokio::io::Result<()> {
        info!("Listening on {:?}", self.listener.local_addr());

        let mut id: SessionId = 0;

        loop {
            let (stream, _) = self.listener.accept().await?;
            self.event_tx
                .send(Event::CreateSessionRequest { stream, id })
                .await
                .unwrap();
            id += 1;
        }
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        common::runtime().block_on(self.discovery.remove_service_from_discovery());
    }
}
