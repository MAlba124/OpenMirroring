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

use log::{debug, error, warn};
// TODO: Fork simple_mdns to be able to have more controll over ttl and stuff
use simple_mdns::InstanceInformation;

pub async fn discover() {
    let discovery = simple_mdns::async_discovery::ServiceDiscovery::new(
        InstanceInformation::new(String::new()),
        "_fcast._tcp.local",
        60,
    )
    .unwrap();

    if let Err(err) = crate::tx!()
        // .send(crate::Event::ReceiverAvailable(
        // "OpenMirroring-test".to_owned(),
        // 10.0.2.2
        // ))
        // Send a test receiver because emulator can't discover any by itself
        .send(crate::Event::ReceiverAvailable {
            name: "OpenMirroring-test".to_owned(),
            addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 2), 46899)),
        })
        .await
    {
        error!("Failed to send ReceiverAvailable: {err}");
    }

    // TODO: remove when they expire
    let mut seen_instances = HashSet::<InstanceInformation>::new();

    debug!("Starting mDNS service discovery");

    loop {
        let services = discovery.get_known_services().await;

        for service in services.into_iter() {
            if seen_instances.contains(&service) {
                continue;
            }

            debug!("Found FCast receiver: {service:?}");

            let mut addresses = service.get_socket_addresses().collect::<Vec<SocketAddr>>();

            if addresses.is_empty() {
                warn!("FCast receiver with no addresses, adding localhost");
                addresses.push(SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::LOCALHOST,
                    46899,
                )));
            }

            // if let Err(err) = crate::tx!()
                // .send(crate::Event::ReceiverAvailable(crate::Receiver {
                // name: service.unescaped_instance_name(),
                // addresses,
                // }))
                // .send(crate::Event::ReceiverAvailable(
                //     service.unescaped_instance_name(),
                // ))
                // .await
            // {
            //     error!("Failed to send ReceiverAvailable: {err}");
            // }

            seen_instances.insert(service);
        }

        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
}

// TODO: use stream
// use async_stream::stream;

// use futures_core::stream::Stream;
// use futures_util::stream::StreamExt;

// pub fn discover() -> impl Stream<Item = crate::Event> {
//     stream! {
//         let discovery = simple_mdns::async_discovery::ServiceDiscovery::new(
//             InstanceInformation::new(String::new()),
//             "_fcast._tcp.local",
//             60,
//         )
//         .unwrap();

//         // TODO: remove when they expire
//         let mut seen_instances = HashSet::<InstanceInformation>::new();

//         debug!("Starting mDNS service discovery");

//         loop {
//             let services = discovery.get_known_services().await;

//             for service in services.into_iter() {
//                 if seen_instances.contains(&service) {
//                     continue;
//                 }

//                 debug!("Found FCast receiver: {service:?}");

//                 let mut addresses = service.get_socket_addresses().collect::<Vec<SocketAddr>>();

//                 if addresses.is_empty() {
//                     warn!("FCast receiver with no addresses, adding localhost");
//                     addresses.push(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 46899)));
//                 }

//                 yield crate::Event::CaptureStarted;
//                 // if let Err(err) = tx
//                 //     .send(crate::Event::ReceiverAvailable(crate::Receiver {
//                 //         name: service.unescaped_instance_name(),
//                 //         addresses,
//                 //     }))
//                 //     .await
//                 // {
//                 //     error!("Failed to send ReceiverAvailable: {err}");
//                 // }

//                 seen_instances.insert(service);
//             }

//             tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
//         }
//     }
// }
