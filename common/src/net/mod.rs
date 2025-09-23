use std::net::{IpAddr, Ipv4Addr};

#[cfg(target_os = "windows")]
mod win;

#[cfg(not(target_os = "windows"))]
mod rest;

pub fn get_all_ip_addresses() -> Vec<IpAddr> {
    #[cfg(target_os = "windows")]
    {
        win::get_all_ip_addresses()
    }

    #[cfg(not(target_os = "windows"))]
    {
        rest::get_all_ip_addresses()
    }
}

pub fn get_default_ipv4_addr() -> Ipv4Addr {
    let addrs = get_all_ip_addresses();
    for addr in addrs {
        if let IpAddr::V4(v4) = addr
            && !v4.is_loopback()
        {
            return v4;
        }
    }

    Ipv4Addr::LOCALHOST
}
