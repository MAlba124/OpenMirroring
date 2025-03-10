#[inline]
const fn hex_decode(hex: char) -> u8 {
    match hex {
        '0' => 0x0,
        '1' => 0x1,
        '2' => 0x2,
        '3' => 0x3,
        '4' => 0x4,
        '5' => 0x5,
        '6' => 0x6,
        '7' => 0x7,
        '8' => 0x8,
        '9' => 0x9,
        'A' => 0xA,
        'B' => 0xB,
        'C' => 0xC,
        'D' => 0xD,
        'E' => 0xE,
        'F' => 0xF,
        _ => unreachable!(),
    }
}

#[inline]
const fn percent_decode(n1: char, n2: char) -> u8 {
    (hex_decode(n1) << 4) + hex_decode(n2)
}

pub fn decode_path(s: &str) -> Result<String, std::string::FromUtf8Error> {
    let mut res: Vec<u8> = Vec::new();
    let s = s.bytes().collect::<Vec<u8>>();
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'%' if i + 2 < s.len() => {
                let decoded = percent_decode(s[i + 1] as char, s[i + 2] as char);
                i += 3;
                res.push(decoded);
            }
            b'%' => panic!("Invalid percent encoded value"),
            _ => {
                res.push(s[i]);
                i += 1;
            }
        }
    }

    String::from_utf8(res)
}

#[cfg(test)]
mod tests {
    use quickcheck_macros::quickcheck;

    use super::*;

    #[inline]
    const fn hex_encode(b: u8) -> char {
        match b {
            0x0 => '0',
            0x1 => '1',
            0x2 => '2',
            0x3 => '3',
            0x4 => '4',
            0x5 => '5',
            0x6 => '6',
            0x7 => '7',
            0x8 => '8',
            0x9 => '9',
            0xA => 'A',
            0xB => 'B',
            0xC => 'C',
            0xD => 'D',
            0xE => 'E',
            0xF => 'F',
            _ => unreachable!(),
        }
    }

    #[inline]
    const fn percent_encode(b: u8) -> [char; 3] {
        ['%', hex_encode(b >> 4), hex_encode(b & 0x0F)]
    }

    /// Percent-encode a given path to valid URL.
    ///
    /// Does not encode '/'.
    fn encode_path(s: &str) -> String {
        let mut res = String::new();

        for ch in s.chars() {
            let mut buf = [0; 4];

            let enc = ch.encode_utf8(&mut buf);

            if enc.len() == 1 {
                match ch {
                    '!' | '#' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ':' | ';'
                    | '=' | '?' | '@' | '[' | ']' | ' ' | '\"' | '%' | '-' | '.' | '<' | '>'
                    | '\\' | '^' | '_' | '`' | '{' | '|' | '}' | '~' => {
                        let encoded = percent_encode(buf[0]);
                        for c in encoded {
                            res.push(c);
                        }
                    }
                    _ => res.push(ch),
                }
            } else
            /* Handle non ASCII */
            {
                let len = enc.len();
                for b in buf[0..len].iter() {
                    for c in percent_encode(*b) {
                        res.push(c)
                    }
                }
            }
        }

        res
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(percent_encode(0x00), ['%', '0', '0']);
        assert_eq!(percent_encode(0xF0), ['%', 'F', '0']);
        assert_eq!(percent_encode(0x0F), ['%', '0', 'F']);
        assert_eq!(percent_encode(0xAB), ['%', 'A', 'B']);
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode('0', '0'), 0x00);
        assert_eq!(percent_decode('F', '0'), 0xF0);
        assert_eq!(percent_decode('0', 'F'), 0x0F);
        assert_eq!(percent_decode('A', 'B'), 0xAB);
    }

    #[test]
    fn encode_no_reserved() {
        assert_eq!(encode_path("helloworld"), "helloworld".to_owned());
    }

    #[test]
    fn encode_reserved() {
        assert_eq!(encode_path("!"), "%21".to_owned());
        assert_eq!(
            encode_path("Hello, World!"),
            "Hello%2C%20World%21".to_owned()
        );
    }

    #[test]
    fn encode_utf8() {
        // import urllib.parse;urllib.parse.quote_plus("æøå")
        assert_eq!(encode_path("æøå"), "%C3%A6%C3%B8%C3%A5".to_owned());
    }

    #[test]
    fn decode_no_encoded() {
        assert_eq!(decode_path("helloworld"), Ok("helloworld".to_owned()));
    }

    #[test]
    fn decode_encoded() {
        assert_eq!(decode_path("%21"), Ok("!".to_owned()));
        assert_eq!(
            decode_path("Hello%2C%20World%21"),
            Ok("Hello, World!".to_owned())
        );
        assert_eq!(decode_path("%20%20%20%20"), Ok("    ".to_owned()));
    }

    #[test]
    fn decode_encoded_utf8() {
        assert_eq!(decode_path("%C3%A6%C3%B8%C3%A5"), Ok("æøå".to_owned()));
    }

    #[quickcheck]
    // QUICKCHECK_TESTS=100000 cargo test
    fn encode_decode(s: String) -> bool {
        let res = decode_path(&encode_path(&s));
        Ok(s) == res
    }
}
