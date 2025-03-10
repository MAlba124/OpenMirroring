use std::net::{Ipv4Addr, Ipv6Addr};

#[cfg(target_os = "windows")]
mod win;

#[cfg(not(target_os = "windows"))]
mod rest;

pub enum Addr {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

pub fn get_all_ip_addresses() -> Vec<Addr> {
    #[cfg(target_os = "windows")]
    { win::get_all_ip_addresses() }

    #[cfg(not(target_os = "windows"))]
    { rest::get_all_ip_addresses() }
}
