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

use std::net::{Ipv4Addr, SocketAddr};
use std::{collections::HashSet, net::SocketAddrV4};

use anyhow::Result;
use log::{debug, error, warn};
// TODO: Fork simple_mdns to be able to have more controll over ttl and stuff
use simple_mdns::InstanceInformation;

pub async fn discover() -> Result<()> {
    debug!("Starting mDNS service discovery");

    let discovery = simple_mdns::async_discovery::ServiceDiscovery::new(
        InstanceInformation::new(String::new()),
        "_fcast._tcp.local",
        60,
        // TODO: remove when they expire
        async move |service| {
            let mut addresses = service.get_socket_addresses().collect::<Vec<SocketAddr>>();

            if addresses.is_empty() {
                warn!("FCast receiver with no addresses, adding localhost");
                addresses.push(SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::LOCALHOST,
                    46899,
                )));
            }

            if let Err(err) = crate::tx!()
                .send(crate::Event::ReceiverAvailable {
                    name: "OpenMirroring-test".to_owned(),
                    addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 2), 46899)),
                })
                .await
            {
                error!("Failed to send ReceiverAvailable: {err}");
            }
        },
    )?;

    if let Err(err) = crate::tx!()
        .send(crate::Event::ReceiverAvailable {
            name: "OpenMirroring-test".to_owned(),
            addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 2), 46899)),
        })
        .await
    {
        error!("Failed to send ReceiverAvailable: {err}");
    }

    Ok(())
}
