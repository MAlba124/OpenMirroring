use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use tokio::sync::mpsc;
use tokio::task;

const IPV4_MDNS = "224.0.0.251"
const IPV6_MDNS = "ff02::fb"
const MDNS_PORT = 5353
const FORCE_UNICAST_RESPONSES: bool = false

#[derive(Debug)]
pub struct Config{
    iface: Option<IpAddr>,
    log_empty_responses: bool,
    //logger should be here
}

pub struct Server{
    config: Arc<Config>,
    ipv4_list: Option<UdpSocket>,
    ipv6_list: Option<UdpSocket>,
    shutdown: Arc<AtomicBool>,
    shutdown_ch: Sender<()>,
}
impl Server{
    pub async fn new(config: Arc<Config>) -> Result<self, String>{
        let ipv4_list = UdpSocket::bind(IPV4_MDNS).ok();
        let ipv6_list = UdpSocket::bind(IPV6_MDNS).ok();

        if ipv4_list.is_none() && ipv6_list.is_none(){
            return Err("No multicast listeners could be started".to_string());
        }

        let (shutdown_tx, _shutdown_rx) = mpsc::channel(1);

        let server = Server{
            config,
            ipv4_list,
            ipv6_list,
            shutdown_ch: shutdown:tx,
        };

        if let Some(ref socket) = server.ipv4_list{
            let sock_clone = socket.try_clone().expect("Failed to clone socket");
            task::spawn(Self::recv(sock_clone));
        }

        if let Some(ref socket) = server.ipv6_list{
            let sock_clone = socket.try_clone().expect("Failed to clone socket");
            task::spawn(Self::recv(sock_clone));
        }

        Ok(server)
    }

    async fn recv(socket: UdpSocket){
        let mut buf = [0u8; 65536];

        loop{
            if self.shutdown.load(Ordering::SeqCst){
                break;
            }

            match socket.recv_from(&mut buf).await{
                Ok((n, from)) => {
                    if let Err(err) = self.parse_packet(&buf[..n], from).await{
                        error!("Failed to handle query: {}", err);
                    }
                }
                Err(err) => {
                    error!("Failed to read from socket: {}", err);
                    continue;
                }
            }
        }
    }

    async fn parse_packet(&self, packet: &[u8], from: SocketAddr) -> Result<(), String>{
        match Message::from_vec(packet){
            Ok(msg) => {
                self.handle_query(&msg, from).await
            }
            Err(err) => {
                error!("Failed to unpack packet: {}", err);
                Err(format!("Failed to unpack packet: {}", err))
            }
        }
    }
}
