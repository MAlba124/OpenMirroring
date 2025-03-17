use std::{error::Error, fmt::Display, io::Read, net::TcpStream};

#[derive(Debug, PartialEq, Eq)]
pub enum HttpError {
    StartLineEmpty,
    InvalidRequestMethod,
    StartLineMissingTarget,
    InvalidRequestTarget,
    StartLineMissingVersion,
    StartLineInvalidVersion,
    InvalidHeaderKey,
    InvalidHeaderValue,
    Read,
    InvalidRequest,
    InvalidContentLength,
}

impl Error for HttpError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

impl Display for HttpError {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RequestMethod {
    Get,
    Post,
    Head,
    Delete,
    Put,
    Connect,
    Options,
    Trace,
    Patch,
}

impl RequestMethod {
    pub fn from_slice(method: &[u8]) -> Option<Self> {
        match method {
            b"GET" => Some(Self::Get),
            b"POST" => Some(Self::Post),
            b"HEAD" => Some(Self::Head),
            b"DELETE" => Some(Self::Delete),
            b"PUT" => Some(Self::Put),
            b"CONNECT" => Some(Self::Connect),
            b"OPTIONS" => Some(Self::Options),
            b"TRACE" => Some(Self::Trace),
            b"PATCH" => Some(Self::Patch),
            _ => None,
        }
    }

    pub fn to_str(&self) -> &'static str {
        match self {
            RequestMethod::Get => "GET",
            RequestMethod::Post => "POST",
            RequestMethod::Head => "HEAD",
            RequestMethod::Delete => "DELETE",
            RequestMethod::Put => "PUT",
            RequestMethod::Connect => "CONNECT",
            RequestMethod::Options => "OPTIONS",
            RequestMethod::Trace => "TRACE",
            RequestMethod::Patch => "PATCH",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum HttpVersion {
    ZeroDotNine,
    One,
    OneDotOne,
    Two,
    Three,
}

impl HttpVersion {
    pub fn from_slice(version: &[u8]) -> Option<Self> {
        match version {
            b"HTTP/0.9" => Some(Self::ZeroDotNine),
            b"HTTP/1.0" => Some(Self::One),
            b"HTTP/1.1" => Some(Self::OneDotOne),
            b"HTTP/2" => Some(Self::Two),
            b"HTTP/3" => Some(Self::Three),
            _ => None,
        }
    }

    pub fn to_str(&self) -> &'static str {
        match self {
            HttpVersion::ZeroDotNine => "HTTP/0.9",
            HttpVersion::One => "HTTP/1.0",
            HttpVersion::OneDotOne => "HTTP/1.1",
            HttpVersion::Two => "HTTP/2",
            HttpVersion::Three => "HTTP/3",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RequestStartLine {
    pub method: RequestMethod,
    pub target: String,
    pub version: HttpVersion,
}

impl RequestStartLine {
    /// Takes in a start line buffer with the "\r\n" suffix removed and returns the parsed value
    pub fn parse(buf: &[u8]) -> Result<Self, HttpError> {
        if buf.is_empty() {
            return Err(HttpError::StartLineEmpty);
        }

        let mut i = 0;
        // Method
        while i < buf.len() && buf[i] != b' ' {
            i += 1;
        }

        let method =
            RequestMethod::from_slice(&buf[0..i]).ok_or(HttpError::InvalidRequestMethod)?;

        i += 1;

        if i >= buf.len() {
            return Err(HttpError::StartLineMissingTarget);
        }

        let mut j = i;
        // Target
        while j < buf.len() && buf[j] != b' ' {
            j += 1;
        }

        let target =
            String::from_utf8(buf[i..j].to_vec()).map_err(|_| HttpError::InvalidRequestTarget)?;

        j += 1;

        if j >= buf.len() {
            return Err(HttpError::StartLineMissingVersion);
        }

        let version =
            HttpVersion::from_slice(&buf[j..]).ok_or(HttpError::StartLineInvalidVersion)?;

        Ok(Self {
            method,
            target,
            version,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Header {
    pub key: String,
    pub value: String,
}

impl Header {
    /// Takes in a header buffer with the "\r\n" suffix removed and returns the parsed value
    pub fn parse(header: &[u8]) -> Result<Self, HttpError> {
        let mut i = 0;
        // Key
        while i < header.len() && header[i] != b':' && header[i] != b' ' {
            i += 1;
        }

        if header[i] == b' ' {
            return Err(HttpError::InvalidHeaderKey);
        }

        let key =
            String::from_utf8(header[0..i].to_vec()).map_err(|_| HttpError::InvalidHeaderKey)?;

        if key.is_empty() {
            return Err(HttpError::InvalidHeaderKey);
        }

        i += 2;

        // Check that there is a space between the <key>: and the <value>
        if i > header.len() || header[i - 1] != b' ' {
            return Err(HttpError::InvalidHeaderValue);
        }

        let value =
            String::from_utf8(header[i..].to_vec()).map_err(|_| HttpError::InvalidHeaderValue)?;

        if value.is_empty() {
            return Err(HttpError::InvalidHeaderValue);
        }

        Ok(Self { key, value })
    }

    pub fn serialize_with_crlf_into(&self, buf: &mut Vec<u8>) {
        // let mut buf = Vec::new();
        buf.extend_from_slice(self.key.as_bytes());
        buf.extend_from_slice(b": ");
        buf.extend_from_slice(self.value.as_bytes());
        buf.extend_from_slice(b"\r\n");
        // buf
    }
}

#[inline]
fn extract_line(buf: &[u8]) -> Option<&[u8]> {
    let mut i = 0;
    while i < buf.len() && buf[i] != b'\r' {
        i += 1;
    }

    if i + 1 >= buf.len() || buf[i + 1] != b'\n' {
        return None;
    }

    Some(&buf[0..i])
}

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub start_line: RequestStartLine,
    pub headers: Vec<Header>,
    pub body: Option<Vec<u8>>,
}

impl Request {
    pub fn parse(buf: &[u8]) -> Result<Self, HttpError> {
        let start_line_buf = extract_line(buf).ok_or(HttpError::InvalidRequest)?;
        let start_line = RequestStartLine::parse(start_line_buf)?;

        let mut headers: Vec<Header> = Vec::new();
        let mut content_length: Option<usize> = None;

        let mut i = start_line_buf.len() + 2;
        while i < buf.len() && buf[i] != b'\r' {
            let header_buf = extract_line(&buf[i..]).ok_or(HttpError::InvalidRequest)?;
            let header = Header::parse(header_buf)?;

            if header.key.as_str() == "Content-Length" {
                content_length = Some(
                    header
                        .value
                        .parse::<usize>()
                        .map_err(|_| HttpError::InvalidContentLength)?,
                );
            }

            headers.push(header);

            i += header_buf.len() + 2;
        }

        if i + 1 >= buf.len() || buf[i + 1] != b'\n' {
            return Err(HttpError::InvalidRequest);
        }

        i += 2;

        let mut body = None;

        if let Some(content_length) = content_length {
            if content_length > 0 {
                if i + content_length > buf.len() {
                    return Err(HttpError::InvalidContentLength);
                }

                body = Some(buf[i..i + content_length].to_vec());
            }
        }

        Ok(Self {
            start_line,
            headers,
            body,
        })
    }

    pub fn read_from_tcp_stream(stream: &mut TcpStream) -> Result<Self, HttpError> {
        let mut request_buf = Vec::new();
        let mut buf = [0; 4096];
        loop {
            let bytes_read = stream.read(&mut buf).map_err(|_| HttpError::Read)?;
            request_buf.extend_from_slice(&buf[0..bytes_read]);
            if bytes_read < buf.len() {
                break;
            }
        }
        Self::parse(&request_buf)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StatusCode {
    Ok,
    BadRequest,
    NotFound,
    InternalServerError,
    NotImplemented,
}

impl StatusCode {
    pub fn to_str(&self) -> &'static str {
        match self {
            StatusCode::Ok => "200 OK",
            StatusCode::BadRequest => "400 Bad Request",
            StatusCode::NotFound => "404 Not Found",
            StatusCode::InternalServerError => "500 Internal Server Error",
            StatusCode::NotImplemented => "501 Not Implemented",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResponseStartLine {
    pub version: HttpVersion,
    pub status: StatusCode,
}

impl ResponseStartLine {
    pub fn serialize_with_crlf_into(&self, buf: &mut Vec<u8>) {
        // let mut buf = Vec::new();
        buf.extend_from_slice(self.version.to_str().as_bytes());
        buf.push(b' ');
        buf.extend_from_slice(self.status.to_str().as_bytes());
        buf.extend_from_slice(b"\r\n");

        // buf
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Response<'a> {
    pub start_line: ResponseStartLine,
    pub headers: Vec<Header>,
    pub body: Option<&'a [u8]>,
}

impl Response<'_> {
    pub fn serialize_into(&self, buf: &mut Vec<u8>) {
        // let mut buf = Vec::new();
        self.start_line.serialize_with_crlf_into(buf);
        // buf.extend_from_slice(&self.start_line.serialize_with_crlf());

        for header in &self.headers {
            header.serialize_with_crlf_into(buf);
            // buf.extend_from_slice(&header.serialize_with_crlf());
        }

        buf.extend_from_slice(b"\r\n");

        if let Some(body) = &self.body {
            buf.extend_from_slice(body);
        }

        // buf
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    macro_rules! req_start_line {
        ($method:ident, $target:expr, $version:ident) => {
            RequestStartLine {
                method: RequestMethod::$method,
                target: $target.to_owned(),
                version: HttpVersion::$version,
            }
        };
    }

    macro_rules! header {
        ($key:expr, $value:expr) => {
            Header {
                key: $key.to_owned(),
                value: $value.to_owned(),
            }
        };
    }

    macro_rules! request {
        ($start_line:expr, $headers:expr, $body:expr) => {
            Request {
                start_line: $start_line,
                headers: $headers,
                body: $body,
            }
        };
    }

    #[test]
    fn parse_start_line() {
        assert_eq!(
            RequestStartLine::parse(b"GET / HTTP/0.9"),
            Ok(req_start_line!(Get, "/", ZeroDotNine))
        );
        assert_eq!(
            RequestStartLine::parse(b"POST / HTTP/0.9"),
            Ok(req_start_line!(Post, "/", ZeroDotNine))
        );
        assert_eq!(
            RequestStartLine::parse(b"GET /index.html HTTP/1.0"),
            Ok(req_start_line!(Get, "/index.html", One))
        );
    }

    #[test]
    fn parse_start_line_empty() {
        assert_eq!(RequestStartLine::parse(b""), Err(HttpError::StartLineEmpty));
    }

    #[test]
    fn parse_start_missing_target() {
        assert_eq!(
            RequestStartLine::parse(b"GET"),
            Err(HttpError::StartLineMissingTarget)
        );
    }

    #[test]
    fn parse_start_missing_version() {
        assert_eq!(
            RequestStartLine::parse(b"GET /"),
            Err(HttpError::StartLineMissingVersion)
        );
    }

    #[test]
    fn parse_start_line_invalid_method() {
        assert_eq!(
            RequestStartLine::parse(b"POKE / HTTP/1.0"),
            Err(HttpError::InvalidRequestMethod)
        );
        assert_eq!(
            RequestStartLine::parse(b"OPTION / HTTP/1.0"),
            Err(HttpError::InvalidRequestMethod)
        );
        assert_eq!(
            RequestStartLine::parse(b"get / HTTP/1.0"),
            Err(HttpError::InvalidRequestMethod)
        );
    }

    #[test]
    fn parse_start_line_invalid_version() {
        assert_eq!(
            RequestStartLine::parse(b"GET / HTTP/0.8"),
            Err(HttpError::StartLineInvalidVersion)
        );
        assert_eq!(
            RequestStartLine::parse(b"GET / HTTP/1.8"),
            Err(HttpError::StartLineInvalidVersion)
        );
        assert_eq!(
            RequestStartLine::parse(b"GET / HTTP"),
            Err(HttpError::StartLineInvalidVersion)
        );
        assert_eq!(
            RequestStartLine::parse(b"GET / 1234"),
            Err(HttpError::StartLineInvalidVersion)
        );
    }

    #[test]
    fn parse_header() {
        assert_eq!(Header::parse(b"Key: Value"), Ok(header!("Key", "Value")),);
        assert_eq!(
            Header::parse(b"Content-Length: 100"),
            Ok(header!("Content-Length", "100")),
        );
        assert_eq!(
            Header::parse(b"Host: example.com"),
            Ok(header!("Host", "example.com")),
        );
    }

    #[test]
    fn parse_header_invalid_header_key() {
        assert_eq!(
            Header::parse(b"Key : Value"),
            Err(HttpError::InvalidHeaderKey),
        );
        assert_eq!(Header::parse(b": Value"), Err(HttpError::InvalidHeaderKey),);
    }

    #[test]
    fn parse_header_invalid_value() {
        assert_eq!(
            Header::parse(b"Key:Value"),
            Err(HttpError::InvalidHeaderValue),
        );
        assert_eq!(Header::parse(b"Key: "), Err(HttpError::InvalidHeaderValue),);
    }

    #[test]
    fn parse_request() {
        assert_eq!(
            Request::parse(
                b"GET / HTTP/1.0\r\n\
              Content-Length: 0\r\n\
              \r\n"
            ),
            Ok(request!(
                req_start_line!(Get, "/", One),
                vec![header!("Content-Length", "0")],
                None
            ))
        );
        assert_eq!(
            Request::parse(
                b"GET / HTTP/1.0\r\n\
              Content-Length: 0\r\n\
              Key: Value\r\n\
              \r\n"
            ),
            Ok(request!(
                req_start_line!(Get, "/", One),
                vec![header!("Content-Length", "0"), header!("Key", "Value"),],
                None
            ))
        );
        assert_eq!(
            Request::parse(
                b"POST / HTTP/1.0\r\n\
              Content-Length: 11\r\n\
              Content-Type: application/json\r\n\
              \r\n\
              {\"json\": 1}"
            ),
            Ok(request!(
                req_start_line!(Post, "/", One),
                vec![
                    header!("Content-Length", "11"),
                    header!("Content-Type", "application/json")
                ],
                Some(b"{\"json\": 1}".to_vec())
            ))
        );
    }

    #[test]
    fn parse_request_invalid_content_length() {
        assert_eq!(
            Request::parse(
                b"GET / HTTP/1.0\r\n\
              Content-Length: 1\r\n\
              \r\n"
            ),
            Err(HttpError::InvalidContentLength)
        );
        assert_eq!(
            Request::parse(
                b"GET / HTTP/1.0\r\n\
              Content-Length: -1\r\n\
              \r\n"
            ),
            Err(HttpError::InvalidContentLength)
        );
    }

    #[test]
    fn serialize_header() {
        {
            let mut buf = Vec::new();
            Header::parse(b"Key: Value")
                .unwrap()
                .serialize_with_crlf_into(&mut buf);
            assert_eq!(&buf, b"Key: Value\r\n");
        }
        {
            let mut buf = Vec::new();
            Header::parse(b"Content-Length: 123456")
                .unwrap()
                .serialize_with_crlf_into(&mut buf);
            assert_eq!(&buf, b"Content-Length: 123456\r\n");
        }
    }

    #[test]
    fn serialize_response_start_line() {
        {
            let mut buf = Vec::new();
            ResponseStartLine {
                version: HttpVersion::OneDotOne,
                status: StatusCode::Ok,
            }
            .serialize_with_crlf_into(&mut buf);
            assert_eq!(&buf, b"HTTP/1.1 200 OK\r\n");
        }
        {
            let mut buf = Vec::new();
            ResponseStartLine {
                version: HttpVersion::ZeroDotNine,
                status: StatusCode::InternalServerError,
            }
            .serialize_with_crlf_into(&mut buf);
            assert_eq!(&buf, b"HTTP/0.9 500 Internal Server Error\r\n");
        }
    }

    #[test]
    fn serialize_response() {
        {
            let mut buf = Vec::new();
            Response {
                start_line: ResponseStartLine {
                    version: HttpVersion::One,
                    status: StatusCode::Ok,
                },
                headers: vec![header!("Content-Length", "0")],
                body: None,
            }
            .serialize_into(&mut buf);
            assert_eq!(
                &buf,
                b"HTTP/1.0 200 OK\r\n\
                Content-Length: 0\r\n\
                \r\n"
            );
        }
        {
            let mut buf = Vec::new();
            Response {
                start_line: ResponseStartLine {
                    version: HttpVersion::One,
                    status: StatusCode::Ok,
                },
                headers: vec![header!("Content-Length", "10")],
                body: Some(b"AAAAAAAAAA"),
            }
            .serialize_into(&mut buf);
            assert_eq!(
                &buf,
                b"HTTP/1.0 200 OK\r\n\
                Content-Length: 10\r\n\
                \r\n\
                AAAAAAAAAA"
            );
        }
    }

    fn headers_are_same(h1: &[Header], h2: &[Header]) -> bool {
        if h1.len() != h2.len() {
            return false;
        }

        let mut h1_set = std::collections::HashSet::new();

        for h in h1 {
            h1_set.insert(h);
        }

        for h in h2 {
            if !h1_set.contains(h) {
                return false;
            }
        }

        true
    }

    #[test]
    fn headers_are_actually_same() {
        {
            let h1 = vec![header!("1", "1"), header!("2", "2"), header!("3", "3")];
            let h2 = vec![header!("1", "1"), header!("2", "2"), header!("3", "3")];
            assert!(headers_are_same(&h1, &h2));
        }
        {
            let h1 = vec![header!("2", "2"), header!("3", "3"), header!("1", "1")];
            let h2 = vec![header!("1", "1"), header!("2", "2"), header!("3", "3")];
            assert!(headers_are_same(&h1, &h2));
        }
    }

    #[test]
    fn headers_are_not_same() {
        {
            let h1 = vec![header!("1", "2"), header!("2", "2"), header!("3", "3")];
            let h2 = vec![header!("1", "1"), header!("2", "2"), header!("3", "3")];
            assert!(!headers_are_same(&h1, &h2));
        }
        {
            let h1 = vec![header!("3", "3"), header!("1", "1")];
            let h2 = vec![header!("1", "1"), header!("2", "2"), header!("3", "3")];
            assert!(!headers_are_same(&h1, &h2));
        }
    }

    #[test]
    fn read_from_tcp_stream_basic() {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let listener = std::net::TcpListener::bind("127.0.0.1:26969").unwrap();

            ready_tx.send(()).unwrap();

            let (mut stream, _) = listener.accept().unwrap();

            let request = Request::read_from_tcp_stream(&mut stream).unwrap();

            assert_eq!(
                request.start_line,
                RequestStartLine {
                    method: RequestMethod::Get,
                    target: "/".to_string(),
                    version: HttpVersion::One,
                }
            );
            assert!(headers_are_same(
                &request.headers,
                &[
                    header!("user-agent", "test"),
                    header!("accept", "*/*"),
                    header!("host", "127.0.0.1:26969"),
                ]
            ));
            assert!(request.body.is_none());

            let response = Response {
                start_line: ResponseStartLine {
                    version: HttpVersion::One,
                    status: StatusCode::Ok,
                },
                headers: vec![header!("Content-Length", "0")],
                body: None,
            };

            let mut response_buf = Vec::new();
            response.serialize_into(&mut response_buf);

            // stream.write_all(&response.serialize()).unwrap();
            stream.write_all(&response_buf).unwrap();

            stream.shutdown(std::net::Shutdown::Both).unwrap();
        });

        ready_rx.recv().unwrap();

        let respone = ureq::get("http://127.0.0.1:26969/")
            .version(ureq::http::Version::HTTP_10)
            .header("user-agent", "test")
            .call()
            .unwrap();

        assert_eq!(respone.status(), ureq::http::StatusCode::OK);
        assert_eq!(respone.version(), ureq::http::Version::HTTP_10);

        handle.join().unwrap();
    }
}
