use std::fmt::{Display, Formatter};
use std::io;
use std::io::ErrorKind;
use std::str::from_utf8;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const BUF_SIZE: usize = 1024;

const MAX_BUF_SIZE: usize = 4096;

#[derive(Debug, Copy, Clone)]
pub struct Status {
    code: u16,
    reason_phrase: &'static str,
}

impl Status {
    pub const fn new(code: u16, reason_phrase: &'static str) -> Self {
        Self {
            code,
            reason_phrase,
        }
    }

    pub async fn send(self, stream: &mut (impl AsyncWrite + Unpin)) -> crate::Result<()> {
        let response = format!(
            "HTTP/1.1 {} {}\r\ncontent-length: 0\r\n\r\n",
            self.code, self.reason_phrase
        );
        stream.write_all(response.as_bytes()).await.map_err(err!())
    }
}

pub const BAD_GATEWAY: Status = Status::new(502, "Bad Gateway");

pub const GATEWAY_TIMEOUT: Status = Status::new(504, "Gateway Timeout");

#[derive(Debug)]
struct HeaderTooLarge;

impl Display for HeaderTooLarge {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "header size exceeds {} bytes", MAX_BUF_SIZE)
    }
}

impl std::error::Error for HeaderTooLarge {}

pub struct ParseResult {
    pub buf: Vec<u8>,
    // 已读取的数据
    pub domain: String, // 域名
}

enum State {
    Start,
    ParseHeader(usize),
}

// 从 Host 头解析域名
pub async fn parse_domain(stream: &mut (impl AsyncRead + Unpin)) -> crate::Result<ParseResult> {
    let mut buf = Vec::with_capacity(BUF_SIZE);
    unsafe { buf.set_len(buf.capacity()) };

    let mut read = 0;
    let mut state = State::Start;
    loop {
        let n = stream.read(&mut buf[read..]).await.map_err(err!())?;
        if n == 0 {
            return Err(io::Error::from(ErrorKind::UnexpectedEof)).map_err(err!());
        }
        read += n;

        'a: loop {
            match state {
                State::Start => match find_r(0, read, &buf) {
                    Some(pos) => state = State::ParseHeader(pos + 2),
                    None => break,
                },
                State::ParseHeader(mut start) => loop {
                    match find_r(start, read, &buf) {
                        Some(end) => match extract_domain(&buf[start..end]) {
                            Some(domain) => {
                                let domain = from_utf8(domain).map_err(err!())?.trim().to_string();
                                buf.truncate(read);
                                return Ok(ParseResult { domain, buf });
                            }
                            None => start = end + 2,
                        },
                        None => {
                            state = State::ParseHeader(start);
                            break 'a;
                        }
                    }
                },
            }
        }

        if read == buf.capacity() {
            if read < MAX_BUF_SIZE {
                buf.reserve(BUF_SIZE);
                unsafe { buf.set_len(buf.capacity()) };
            } else {
                return Err(HeaderTooLarge).map_err(err!());
            }
        }
    }
}

fn find_r(start: usize, end: usize, s: &[u8]) -> Option<usize> {
    for i in start..end {
        if s[i] == b'\r' {
            return Some(i);
        }
    }
    None
}

fn extract_domain(s: &[u8]) -> Option<&[u8]> {
    let mut s = s.split(|&v| v == b':');
    if eq_host(s.next()?) {
        s.next()
    } else {
        None
    }
}

fn eq_host(s: &[u8]) -> bool {
    s.len() == 4
        && (s[0] | 32) == b'h'
        && (s[1] | 32) == b'o'
        && (s[2] | 32) == b's'
        && (s[3] | 32) == b't'
}
