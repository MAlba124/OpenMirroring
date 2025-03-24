use std::{fmt::Display, net::{Ipv4Addr, Ipv6Addr}};

#[cfg(target_os = "windows")]
mod win;

#[cfg(not(target_os = "windows"))]
mod rest;

pub enum Addr {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

impl Display for Addr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Addr::V4(v4) => write!(f, "{v4}"),
            Addr::V6(v6) => write!(f, "[{v6}]"),
        }
    }
}

pub fn get_all_ip_addresses() -> Vec<Addr> {
    #[cfg(target_os = "windows")]
    {
        win::get_all_ip_addresses()
    }

    #[cfg(not(target_os = "windows"))]
    {
        rest::get_all_ip_addresses()
    }
}
