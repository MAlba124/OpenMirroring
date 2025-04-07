use std::{collections::HashSet, net::SocketAddrV4};
use std::net::{Ipv4Addr, SocketAddr};

use log::{debug, error, warn};
use simple_mdns::InstanceInformation;

pub async fn discover(tx: tokio::sync::mpsc::Sender<crate::Event>) {
    let discovery = simple_mdns::async_discovery::ServiceDiscovery::new(
        InstanceInformation::new(String::new()),
        "_fcast._tcp.local",
        60,
    )
    .unwrap();

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
                addresses.push(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 46899)));
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

            seen_instances.insert(service);
        }

        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
}
