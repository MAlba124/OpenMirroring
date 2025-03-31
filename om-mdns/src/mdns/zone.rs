extern crate hostname;

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use crate::dns::byte_packet::*;

const TTL_DEFAULT: u32 = 120;

trait Zone{
    fn records(&self, q: DnsQuestion) -> Result<Vec<DnsRecord>, String>;
}

#[derive(Debug)]
pub struct MDNSService {
    Instance: String,
    Service: String,
    Domain: String,
    pub HostName: String,
    Port: u32,
    IPs: Vec<IpAddr>,
    TXT: Vec<String>,

    pub serviceAddr: String,
    pub instanceAddr: String,
    pub enumAddr: String,
}

impl MDNSService{
    pub fn records(&self, q: DnsQuestion) ->  Result<Vec<DnsRecord>, String>{
        match q.name.as_str(){
            name if name == self.enumAddr => return self.service_enum(q),
            name if name == self.serviceAddr => return self.service_records(q),
            name if name == self.instanceAddr => return self.instance_records(q),
            name if name == self.HostName => {
                if q.qtype == QueryType::A || q.qtype == QueryType::AAAA{
                    return self.instance_records(q);
                }
                return Ok(Vec::new())
            },
            _ => return Ok(Vec::new())
        }
    }

    fn service_enum(&self, q: DnsQuestion) -> Result<Vec<DnsRecord>, String>{
        match q.qtype{ 
            QueryType::UNKNOWN(_) | QueryType::PTR => {
                let rr = DnsRecord::PTR{
                    hdr: RecordHeader{
                        name: q.name,
                        rr_type: DnsR::PTR,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: 0,
                    },
                    ptr: self.serviceAddr.clone(),
                };
                Ok(vec![rr])
            },
            _ => Err(format!("Unsupported query type")),
        }
    }
    
    fn service_records(&self, q: DnsQuestion) -> Result<Vec<DnsRecord>, String>{
        match q.qtype{
            QueryType::UNKNOWN(_) | QueryType::PTR => {
                let rr = DnsRecord::PTR{
                    hdr: RecordHeader{
                        name: q.name,
                        rr_type: DnsR::PTR,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: 0,
                    },
                    ptr: self.instanceAddr.clone(),
                };
                let mut serv_rec: Vec<DnsRecord> = vec![rr];

                let inst_recs = self.instance_records(DnsQuestion{
                    name: self.instanceAddr.clone(),
                    qtype: QueryType::UNKNOWN(0),
                })?;

                serv_rec.extend(inst_recs);

                Ok(serv_rec)
            }
            _ => Ok(Vec::new()),
        }
    }

    fn instance_records(&self, q: DnsQuestion) -> Result<Vec<DnsRecord>, String>{
        match q.qtype{
            //Any type (255)
            QueryType::UNKNOWN(_) => {
                let mut recs: Vec<DnsRecord> = self.instance_records(DnsQuestion{
                    name: self.instanceAddr.clone(),
                    qtype: QueryType::SRV,
                })?;

                recs.extend(self.instance_records(DnsQuestion{
                    name: self.instanceAddr.clone(),
                    qtype: QueryType::TXT,
                })?);
                Ok(recs)
            }
            QueryType::A => {
                let mut rr:Vec<DnsRecord> = Vec::new();
                for ip in &self.IPs{
                    if let IpAddr::V4(ipv4) = ip{
                        rr.push(DnsRecord::A{
                            hdr: RecordHeader{
                                name: self.HostName.clone(),
                                rr_type: DnsR::A,
                                class: Class::INET,
                                ttl:TTL_DEFAULT,
                                len: 0,
                            },
                            a: *ipv4,
                        });
                    }
                }
                Ok(rr)
            }
            QueryType::AAAA => {
                let mut rr:Vec<DnsRecord> = Vec::new();
                for ip in &self.IPs{
                    if let IpAddr::V6(ipv6) = ip{
                        rr.push(DnsRecord::AAAA{
                            hdr: RecordHeader{
                                name: self.HostName.clone(),
                                rr_type: DnsR::AAAA,
                                class: Class::INET,
                                ttl: TTL_DEFAULT,
                                len: 0,
                            },
                            aaaa: *ipv6,
                        });
                    }
                }
                Ok(rr)
            }
            QueryType::SRV => {
                let srv = DnsRecord::SRV{
                    hdr: RecordHeader{
                        name: q.name,
                        rr_type: DnsR::SRV,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: 0,
                    },
                    priority: 10,
                    weight: 1,
                    port: self.Port as u16,
                    target: self.HostName.clone(),
                };
                let mut recs: Vec<DnsRecord> = vec![srv];

                recs.extend(self.instance_records(DnsQuestion{
                    name: self.instanceAddr.clone(),
                    qtype: QueryType::A,
                })?);

                recs.extend(self.instance_records(DnsQuestion{
                    name: self.instanceAddr.clone(),
                    qtype: QueryType::AAAA,
                })?);
                
                Ok(recs)
            }
            QueryType::TXT => {
                let txt = DnsRecord::TXT{
                    hdr: RecordHeader{
                        name: q.name,
                        rr_type: DnsR::TXT,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: 0,
                    },
                    txt: self.TXT.clone(),
                };

                Ok(vec![txt])
            }
            _ => {
                Ok(Vec::new())
            }
        }
    }
}

pub fn newMDNSService(
    instance: &str,
    service: &str,
    mut domain: &str,
    mut hostname: String,
    port: u32,
    mut ips: Vec<IpAddr>,
    txt: Vec<String>,
) -> Result<MDNSService, String> {
    if instance == ""{
        return Err("Missing service instance name".to_string());
    }
    if service == ""{
        return Err("Missing service name".to_string());
    }
    if port == 0{
        return Err("Missing service port".to_string());
    }

    if domain.is_empty() {
        domain = "local.";
    }
    if let Err(_) = validateFQDN(&domain) {
        return Err("Not a fully qualified domain name".to_string());
    }
    if hostname.is_empty() {
        match hostname::get() {
            Ok(h) => hostname = format!("{}.", h.to_string_lossy()),
            Err(_) => return Err("Could not retrieve hostname".to_string()),
        }
        println!("new hostname: {}", hostname);
    }
    if let Err(_) = validateFQDN(&hostname) {
        return Err("Not a fully qualified hostname".to_string());
    }

    if ips.is_empty() {
        match lookUpIP(&hostname, port, &domain) {
            Ok(addrs) => {
                ips = addrs.iter().map(|addr| addr.ip()).collect();
            }
            Err(_) => return Err("Could not determine IP addresses for host".to_string()),
        }
    }
    Ok(MDNSService {
        Instance: instance.to_string(),
        Service: service.to_string(),
        Domain: domain.to_string(),
        HostName: hostname,
        Port: port,
        IPs: ips,
        TXT: txt,
        serviceAddr: format!("{}.{}.", trimDot(&service), trimDot(&domain)),
        instanceAddr: format!("{}.{}.{}.", instance, trimDot(&service), trimDot(&domain)),
        enumAddr: format!("_services._dns-sd._udp.{}.", trimDot(&domain)),
    })
}

fn lookUpIP(host: &str, port: u32, domain: &str) -> Result<Vec<SocketAddr>, String> {
    let hostWithPort = format!("{}:{}", trimDot(host), port);
    println!("hostname: {}", hostWithPort);
    match hostWithPort.to_socket_addrs() {
        Ok(addrs) => Ok(addrs.collect()),
        Err(_) => {
            let hostDomainWithPort = format!("{}{}:{}", host, trimDot(domain), port);
            println!("hostname with domain: {}", hostDomainWithPort);
            match hostDomainWithPort.to_socket_addrs() {
                Ok(addrs) => Ok(addrs.collect()),
                Err(_) => Err("Failed to retrieve ips".to_string()),
            }
        }
    }
}

fn validateFQDN(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("FQDN must not be blank".to_string());
    }
    if let Some(c) = s.chars().last() {
        if c != '.' {
            return Err("FQDN must end in period".to_string());
        }
    }
    Ok(())
}

fn trimDot(s: &str) -> String {
    s.trim_matches('.').to_string()
}
