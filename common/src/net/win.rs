use windows::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::GetAdaptersAddresses;
use windows::Win32::NetworkManagement::IpHelper::GAA_FLAG_INCLUDE_PREFIX;
use windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_ADDRESSES_LH;
use windows::Win32::Networking::WinSock::AF_INET;
use windows::Win32::Networking::WinSock::AF_INET6;
use windows::Win32::Networking::WinSock::AF_UNSPEC;
use windows::Win32::Networking::WinSock::SOCKADDR_IN;
use windows::Win32::Networking::WinSock::SOCKADDR_IN6;

const AF_INET_VAL: u16 = AF_INET.0;
const AF_INET6_VAL: u16 = AF_INET6.0;

pub fn get_all_ip_addresses() -> Vec<super::Addr> {
    let family = AF_UNSPEC.0 as u32;
    let flag = GAA_FLAG_INCLUDE_PREFIX;

    let mut buffer_length: u32 = 0;
    let ret = unsafe { GetAdaptersAddresses(family, flag, None, None, &mut buffer_length) };

    assert_eq!(ret, ERROR_BUFFER_OVERFLOW.0);

    let mut buffer = vec![0u8; buffer_length as usize];
    let adapter_addresses_ptr = buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;

    let ret = unsafe {
        GetAdaptersAddresses(
            family,
            flag,
            None,
            Some(adapter_addresses_ptr),
            &mut buffer_length,
        )
    };

    assert_eq!(ret, NO_ERROR.0);

    let mut addrs = Vec::new();

    let mut adapter = adapter_addresses_ptr;
    while !adapter.is_null() {
        let adapter_ref: &IP_ADAPTER_ADDRESSES_LH = unsafe { &*adapter };

        let mut unicast = adapter_ref.FirstUnicastAddress;
        while !unicast.is_null() {
            let address = unsafe { (*unicast).Address.lpSockaddr };
            if !address.is_null() {
                let family = unsafe { (*address).sa_family.0 };
                match family {
                    AF_INET_VAL => {
                        let sockaddr_in = unsafe { &*(address as *const SOCKADDR_IN) };
                        let addr_bytes = unsafe { sockaddr_in.sin_addr.S_un.S_addr.to_ne_bytes() };
                        addrs.push(super::Addr::V4(std::net::Ipv4Addr::new(
                            addr_bytes[0],
                            addr_bytes[1],
                            addr_bytes[2],
                            addr_bytes[3],
                        )));
                    }
                    AF_INET6_VAL => {
                        let sockaddr_in6 = unsafe { &*(address as *const SOCKADDR_IN6) };
                        let addr_bytes = unsafe { sockaddr_in6.sin6_addr.u.Byte };
                        addrs.push(super::Addr::V6(std::net::Ipv6Addr::from(addr_bytes)));
                    }
                    _ => (),
                }
            }

            unicast = unsafe { (*unicast).Next };
        }

        adapter = adapter_ref.Next;
    }

    addrs
}
