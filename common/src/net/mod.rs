use std::net::IpAddr;

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
