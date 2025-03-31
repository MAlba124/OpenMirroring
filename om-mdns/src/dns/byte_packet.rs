use std::net::{Ipv4Addr, Ipv6Addr};

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

const TTL_DEFAULT: u32 = 120;

pub struct BytePacketBuffer {
    pub buf: [u8; 512],
    pub pos: usize,
}

impl BytePacketBuffer {
    //This is where the packet goes
    //Pos is for the current position in the packet
    pub fn new() -> BytePacketBuffer {
        BytePacketBuffer {
            buf: [0; 512],
            pos: 0,
        }
    }

    //Returns current position in the buffer
    fn pos(&self) -> usize {
        self.pos
    }

    //Steps the buffer position
    fn step(&mut self, steps: usize) -> Result<()> {
        self.pos += steps;

        Ok(())
    }

    //Changes the current position
    fn seek(&mut self, pos: usize) -> Result<()> {
        self.pos = pos;

        Ok(())
    }

    //Read a byte from current position and step one byte forward
    fn read(&mut self) -> Result<u8> {
        if self.pos >= 512 {
            return Err("End of buffer".into());
        }
        let res = self.buf[self.pos];
        self.pos += 1;

        Ok(res)
    }

    //Getting a specific byte
    fn get(&mut self, pos: usize) -> Result<u8> {
        if pos >= 512 {
            return Err("Position out of bounds".into());
        }
        Ok(self.buf[pos])
    }

    //Getting a range of bytes
    fn get_range(&mut self, start: usize, len: usize) -> Result<&[u8]> {
        if start + len >= 512 {
            return Err("Slice out of bounds".into());
        }
        Ok(&self.buf[start..start + len as usize])
    }

    //Read two bytes and step forward twice
    fn read_u16(&mut self) -> Result<u16> {
        let res = ((self.read()? as u16) << 8) | (self.read()? as u16);

        Ok(res)
    }

    //Read four bytes and step forward four times
    fn read_u32(&mut self) -> Result<u32> {
        let res = ((self.read()? as u32) << 24)
            | ((self.read()? as u32) << 16)
            | ((self.read()? as u32) << 8)
            | (self.read()? as u32);

        Ok(res)
    }

    //Reads domain name from the DNS packet
    fn read_qname(&mut self, outstr: &mut String) -> Result<()> {
        let mut pos = self.pos();
        let mut jumped = false;
        let max_jumps = 5;
        let mut jumps_performed = 0;
        let mut delim = "";
        loop {
            //If the program exceeded jumps threshold, throw an error
            if jumps_performed > max_jumps {
                return Err(format!("Limit of {} jumps exceeded", max_jumps).into());
            }

            //qname always prepends each part with its length
            let len = self.get(pos)?;

            //The two most significant bits of the first two bytes represent if there will be a jump or not
            if (len & 0xC0) == 0xC0 {
                //If this is first time jumping, we step forward twices to skip over the first two
                //bytes
                if !jumped {
                    self.seek(pos + 2)?;
                }

                //Read the second byte of the two
                let b2 = self.get(pos + 1)? as u16;
                //XOR the first (current) byte with 0xC0, to remove the firs two significant bits
                //that are 1 to extract the rest of the byte, shift it by 8 bits and OR with the
                //second byte, which gives us the full location of where to jump
                let offset = (((len as u16) ^ 0xC0) << 8) | b2;
                pos = offset as usize;

                jumped = true;
                jumps_performed += 1;

                continue;
            } else {
                pos += 1;
                //Domain name always ends with an null byte, signifying the end of the domain name
                if len == 0 {
                    break;
                }

                outstr.push_str(delim);
                let str_buffer = self.get_range(pos, len as usize)?;
                outstr.push_str(&String::from_utf8_lossy(str_buffer).to_lowercase());

                delim = ".";

                pos += len as usize;
            }
        }

        if !jumped {
            self.seek(pos)?;
        }

        Ok(())
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResultCode {
    NOERROR = 0,
    FORMERR = 1,
    SERVFAIL = 2,
    NXDOMAIN = 3,
    NOTIMP = 4,
    REFUSED = 5,
}

impl ResultCode {
    pub fn from_num(num: u8) -> ResultCode {
        match num {
            1 => ResultCode::FORMERR,
            2 => ResultCode::SERVFAIL,
            3 => ResultCode::NXDOMAIN,
            4 => ResultCode::NOTIMP,
            5 => ResultCode::REFUSED,
            0 | _ => ResultCode::NOERROR,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DnsHeader {
    pub id: u16,

    pub recursion_desired: bool,
    pub truncated_message: bool,
    pub authoritative_answer: bool,
    pub opcode: u8,
    pub response: bool,

    pub rescode: ResultCode,
    pub checking_disabled: bool,
    pub authed_data: bool,
    pub z: bool,
    pub recursion_available: bool,

    pub questions: u16,
    pub answers: u16,
    pub authoritative_entries: u16,
    pub resource_entries: u16,
}

impl DnsHeader {
    pub fn new() -> DnsHeader {
        DnsHeader {
            id: 0,

            recursion_desired: false,
            truncated_message: false,
            authoritative_answer: false,
            opcode: 0,
            response: false,

            rescode: ResultCode::NOERROR,
            checking_disabled: false,
            authed_data: false,
            z: false,
            recursion_available: false,

            questions: 0,
            answers: 0,
            authoritative_entries: 0,
            resource_entries: 0,
        }
    }

    pub fn read(&mut self, buffer: &mut BytePacketBuffer) -> Result<()> {
        self.id = buffer.read_u16()?;

        let flags = buffer.read_u16()?;
        let a = (flags >> 8) as u8;
        let b = (flags & 0xFF) as u8;
        self.recursion_desired = (a & (1 << 0)) > 0;
        self.truncated_message = (a & (1 << 1)) > 0;
        self.authoritative_answer = (a & (1 << 2)) > 0;
        self.opcode = (a >> 3) & 0x0F;
        self.response = (a & (1 << 7)) > 0;

        self.rescode = ResultCode::from_num(b & 0x0F);
        self.checking_disabled = (b & (1 << 4)) > 0;
        self.authed_data = (b & (1 << 5)) > 0;
        self.z = (b & (1 << 6)) > 0;
        self.recursion_available = (b & (1 << 7)) > 0;

        self.questions = buffer.read_u16()?;
        self.answers = buffer.read_u16()?;
        self.authoritative_entries = buffer.read_u16()?;
        self.resource_entries = buffer.read_u16()?;

        Ok(())
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Hash, Copy)]
pub enum QueryType {
    UNKNOWN(u16),
    A,
    PTR,
    TXT,
    AAAA,
    SRV,
}

impl QueryType {
    pub fn to_num(&self) -> u16 {
        match *self {
            QueryType::UNKNOWN(x) => x,
            QueryType::A => 1,
            QueryType::PTR => 12, 
            QueryType::TXT => 16,
            QueryType::AAAA => 28,
            QueryType::SRV => 33,
        }
    }

    pub fn from_num(num: u16) -> QueryType {
        match num {
            1 => QueryType::A,
            12 => QueryType::PTR,
            16 => QueryType::TXT,
            28 => QueryType::AAAA,
            33 => QueryType::SRV,
            _ => QueryType::UNKNOWN(num),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsQuestion {
    pub name: String,
    pub qtype: QueryType,
}

impl DnsQuestion {
    pub fn new(name: String, qtype: QueryType) -> DnsQuestion {
        DnsQuestion {
            name: name,
            qtype: qtype,
        }
    }

    pub fn read(&mut self, buffer: &mut BytePacketBuffer) -> Result<()> {
        buffer.read_qname(&mut self.name)?;
        self.qtype = QueryType::from_num(buffer.read_u16()?);
        let _ = buffer.read_u16()?;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(dead_code)]
pub enum DnsRecord {
    UNKNOWN {
        hdr: RecordHeader,
        domain: String,
        qtype: u16,
        data_len: u16,
    },
    A {
        hdr: RecordHeader,
        a: Ipv4Addr,
    },
    PTR {
        hdr: RecordHeader,
        ptr: String,
    },
    TXT {
        hdr: RecordHeader,
        txt: Vec<String>,
    },
    AAAA {
        hdr: RecordHeader,
        aaaa: Ipv6Addr,
    },
    SRV {
        hdr: RecordHeader,
        priority: u16,
        weight: u16,
        port: u16,
        target: String,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(dead_code)]
pub enum DnsR{
    A = 1,
    PTR = 12,
    TXT = 16,
    AAAA = 28,
    SRV = 33,
    All = 255,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(dead_code)]
pub enum Class{
    INET,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[allow(dead_code)]
pub struct RecordHeader{
    pub name: String,
    pub rr_type: DnsR,
    pub class: Class,
    pub ttl: u32,
    pub len: u16,
}

impl DnsRecord {
    pub fn read(buffer: &mut BytePacketBuffer) -> Result<DnsRecord> {
        let mut domain = String::new();
        buffer.read_qname(&mut domain)?;

        let qtype_num = buffer.read_u16()?;
        let qtype = QueryType::from_num(qtype_num);
        let _ = buffer.read_u16()?;
        let ttl = buffer.read_u32()?;
        let data_len = buffer.read_u16()?;

        match qtype {
            QueryType::A => {
                let raw_addr = buffer.read_u32()?;
                let addr = Ipv4Addr::new(
                    ((raw_addr >> 24) & 0xFF) as u8,
                    ((raw_addr >> 16) & 0xFF) as u8,
                    ((raw_addr >> 8) & 0xFF) as u8,
                    (raw_addr & 0xFF) as u8,
                );
                Ok(DnsRecord::A {
                    hdr: RecordHeader{
                        name: domain.clone(),
                        rr_type: DnsR::A,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: 0,
                    },
                    a: addr,
                })
            }
            QueryType::PTR => {
                //Might need to change
                let mut domain = String::new();
                buffer.read_qname(&mut domain)?;
                let _qtype = buffer.read_u16()?;
                let _class = buffer.read_u16()?;
                let ttl = buffer.read_u32()?;
                let _data_len = buffer.read_u16()?;
                let mut ptr_name = String::new();
                buffer.read_qname(&mut ptr_name)?;
                //let domain = buffer.read_qname()?;
                Ok(DnsRecord::PTR{
                    hdr: RecordHeader{
                        name: domain.clone(),
                        rr_type: DnsR::PTR,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: 0,
                    },
                    ptr: ptr_name,
                })
            }
            QueryType::TXT => {
                Ok(DnsRecord::TXT{
                    hdr: RecordHeader{
                        name: domain.clone(),
                        rr_type: DnsR::TXT,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: 0,
                    },
                    //Keep it just empty for now
                    txt: Vec::new(),
                })
            }
            QueryType::AAAA => {
                let mut segments = [0u16; 8];
                for segment in segments.iter_mut() {
                    *segment = buffer.read_u16()?;
                }
                Ok(DnsRecord::AAAA{
                    hdr: RecordHeader{
                        name: domain.clone(),
                        rr_type: DnsR::AAAA,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: 0,
                    },
                        aaaa: Ipv6Addr::new(
                            segments[0], segments[1], segments[2], segments[3],
                            segments[4], segments[5], segments[6], segments[7],)
                })
            }
            QueryType::SRV => {
                let pr = buffer.read_u16()?;
                let wh = buffer.read_u16()?;
                let pt = buffer.read_u16()?;
                Ok(DnsRecord::SRV{
                    hdr: RecordHeader{
                        name: domain.clone(),
                        rr_type: DnsR::SRV,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: 0,
                    },
                    priority: pr,
                    weight: wh,
                    port: pt,
                    target: domain,
                })
            }
            QueryType::UNKNOWN(_) => {
                buffer.step(data_len as usize)?;

                Ok(DnsRecord::UNKNOWN {
                    hdr: RecordHeader{
                        name: String::new(),
                        rr_type: DnsR::All,
                        class: Class::INET,
                        ttl: TTL_DEFAULT,
                        len: data_len,
                    },
                    domain: domain,
                    qtype: qtype_num,
                    data_len: data_len,
                })
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct DnsPacket {
    pub header: DnsHeader,
    pub questions: Vec<DnsQuestion>,
    pub answers: Vec<DnsRecord>,
    pub authorities: Vec<DnsRecord>,
    pub resources: Vec<DnsRecord>,
}

impl DnsPacket {
    pub fn new() -> DnsPacket {
        DnsPacket {
            header: DnsHeader::new(),
            questions: Vec::new(),
            answers: Vec::new(),
            authorities: Vec::new(),
            resources: Vec::new(),
        }
    }

    pub fn from_buffer(buffer: &mut BytePacketBuffer) -> Result<DnsPacket> {
        let mut result = DnsPacket::new();
        result.header.read(buffer);

        for _ in 0..result.header.questions {
            let mut question = DnsQuestion::new("".to_string(), QueryType::UNKNOWN(0));
            question.read(buffer)?;
            result.questions.push(question);
        }

        for _ in 0..result.header.answers {
            let rec = DnsRecord::read(buffer)?;
            result.answers.push(rec);
        }

        for _ in 0..result.header.authoritative_entries {
            let rec = DnsRecord::read(buffer)?;
            result.authorities.push(rec);
        }

        for _ in 0..result.header.resource_entries {
            let rec = DnsRecord::read(buffer)?;
            result.resources.push(rec);
        }

        Ok(result)
    }
}
