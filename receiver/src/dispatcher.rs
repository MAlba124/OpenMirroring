// Copyright (C) 2025 Marcus L. Hanestad <marlhan@proton.me>
//
// This file is part of OpenMirroring.
//
// OpenMirroring is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// OpenMirroring is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with OpenMirroring.  If not, see <https://www.gnu.org/licenses/>.

use anyhow::Result;
use log::{debug, info};
use simple_mdns::async_discovery::ServiceDiscovery;
use tokio::net::TcpListener;

use crate::{Event, session::SessionId};

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

        let discovery =
            ServiceDiscovery::new(instance_info, "_fcast._tcp.local", 60, async |_| {}).unwrap();

        Ok(Self {
            listener,
            event_tx,
            discovery,
        })
    }

    // TODO: Websocket listener
    pub async fn run(mut self, mut fin_rx: tokio::sync::oneshot::Receiver<()>) -> Result<()> {
        info!("Listening on {:?}", self.listener.local_addr());

        let mut id: SessionId = 0;

        loop {
            tokio::select! {
                _ = &mut fin_rx => break,
                r = self.listener.accept() => {
                    let (stream, _) = r?;
                    self.event_tx
                        .send(Event::CreateSessionRequest { stream, id })
                        .await?;
                    id += 1;
                }
            }
        }

        debug!("Quitting");

        self.discovery.remove_service_from_discovery().await;

        self.event_tx.send(Event::Quit).await?;

        Ok(())
    }
}
