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
use std::net::SocketAddrV4;
use std::net::{Ipv4Addr, SocketAddr};

use log::{debug, error, warn};
use simple_mdns::async_discovery::ServiceDiscovery;
use simple_mdns::InstanceInformation;

// TODO: remove when expire
pub fn discover(tx: tokio::sync::mpsc::Sender<crate::Event>) -> Result<ServiceDiscovery> {
    debug!("Starting mDNS service discovery");

    Ok(ServiceDiscovery::new(
        InstanceInformation::new(String::new()),
        "_fcast._tcp.local",
        60,
        move |service| {
            let tx = tx.clone();
            async move {
                let mut addresses = service.get_socket_addresses().collect::<Vec<SocketAddr>>();

                if addresses.is_empty() {
                    warn!("FCast receiver with no addresses, adding localhost");
                    addresses.push(SocketAddr::V4(SocketAddrV4::new(
                        Ipv4Addr::LOCALHOST,
                        46899,
                    )));
                }

                if let Err(err) = tx
                    .send(crate::Event::ReceiverAvailable(crate::Receiver {
                        name: service.unescaped_instance_name(),
                        addresses,
                    }))
                    .await
                {
                    error!("Failed to send ReceiverAvailable: {err}");
                }
            }
        },
    )?)
}
