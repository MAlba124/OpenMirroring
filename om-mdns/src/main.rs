mod dns;
mod mdns;
use dns::byte_packet::*;
use mdns::zone::*;
use std::fs::File;
use std::io::Read;

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

fn main() -> Result<()>{
    /*
    let mut f = File::open("query_packet.txt")?;
    let mut buffer = BytePacketBuffer::new();
    f.read(&mut buffer.buf)?;

    let packet = DnsPacket::from_buffer(&mut buffer)?;
    println!("{:#?}", packet.header);

    for q in packet.questions{
        println!("{:#?}", q);
    }
    for rec in packet.answers {
        println!("{:#?}", rec);
    }
    for rec in packet.authorities {
        println!("{:#?}", rec);
    }
    for rec in packet.resources {
        println!("{:#?}", rec);
    }
*/
    let service = "_http._tcp";
    let instance = "MyInstance";
    let domain = "local.";
    let hostname = String::new();
    let port = 5353;
    let ips = Vec::new();
    let txt = Vec::new();

    let mdns_service = newMDNSService(instance, service, domain, hostname, port, ips, txt).expect("Failed to create MDNSService");

    let question = DnsQuestion{
        name: mdns_service.serviceAddr.clone(),
        qtype: QueryType::PTR,
    };

    match mdns_service.records(question){
        Ok(records) => {
            println!("Returned records: {:#?}", records);
        }
        Err(e) => {
            println!("Error: {}", e);
        }
    }

    Ok(())
}
