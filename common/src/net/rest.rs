use std::net::IpAddr;

pub(crate) fn get_all_ip_addresses() -> Vec<IpAddr> {
    let mut addrs = Vec::new();
    for iface in pnet_datalink::interfaces() {
        for ip in iface.ips {
            match ip {
                ipnetwork::IpNetwork::V4(v4) => addrs.push(IpAddr::V4(v4.ip())),
                ipnetwork::IpNetwork::V6(v6) => addrs.push(IpAddr::V6(v6.ip())),
            }
        }
    }
    addrs
}
