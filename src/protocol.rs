use std::io;
use std::io::ErrorKind;

use log::debug;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

// 服务端发给客户端的转发请求
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub key: Vec<u8>,
    // 此转发在服务端的唯一标识
    pub domain: String, // 转发对应的域名
}

impl Request {
    pub fn new(key: Vec<u8>, domain: String) -> Self {
        Self { key, domain }
    }
}

// 服务端客户端之间的协议
#[derive(Debug, Serialize, Deserialize)]
pub enum Protocol {
    // 客户端注册
    Register {
        // 客户端想要转发的域名
        domains: Vec<String>,
    },

    // 客户端注册成功
    Ok,

    // 客户端注册失败，至少一个域名已被其他客户端使用
    Error,

    // 转发请求
    Request(Request),

    // 转发响应, 表示当前连接接受服务端标识为 key 的转发
    Response {
        key: Vec<u8>,
    },

    Ping,

    Pong,
}

impl Protocol {
    // 发送, 非取消安全, 不能用于 tokio::select!
    pub async fn send(&self, stream: &mut (impl AsyncWrite + Unpin)) -> crate::Result<()> {
        debug!("send {:?}", self);
        let len = bincode::serialized_size(self).map_err(err!())?;
        debug_assert!(len + 2 < u16::MAX as u64);

        let mut buf = Vec::with_capacity(len as usize + 2);
        unsafe { buf.set_len(buf.capacity()) };
        buf[..2].copy_from_slice(&(len as u16).to_be_bytes());
        buf[2..].copy_from_slice(&bincode::serialize(self).map_err(err!())?);
        stream.write_all(&buf).await.map_err(err!("write_all"))
    }
}

// 读取状态
enum State {
    ReadLen { buf: [u8; 2], read: usize },
    ReadPayload { buf: Vec<u8>, read: usize },
}

impl State {
    fn new() -> Self {
        Self::ReadLen {
            buf: [0; 2],
            read: 0,
        }
    }
}

pub struct Receiver {
    state: State,
}

impl Receiver {
    pub fn new() -> Self {
        Self {
            state: State::new(),
        }
    }

    // 接收消息, 取消安全, 可用于 tokio::select!
    pub async fn recv(
        &mut self,
        stream: &mut (impl AsyncRead + Unpin),
    ) -> crate::Result<Option<Protocol>> {
        loop {
            match &mut self.state {
                State::ReadLen { buf, read } => {
                    let n = stream.read(&mut buf[*read..]).await.map_err(err!())?;
                    *read += n;
                    if *read == 2 {
                        let len = u16::from_be_bytes([buf[0], buf[1]]);
                        let mut buf = Vec::with_capacity(len as usize);
                        unsafe { buf.set_len(buf.capacity()) };
                        self.state = State::ReadPayload { buf, read: 0 }
                    } else if n == 0 {
                        return if *read == 0 {
                            Ok(None)
                        } else {
                            Err(io::Error::from(ErrorKind::UnexpectedEof)).map_err(err!())
                        };
                    }
                }
                State::ReadPayload { buf, read } => {
                    let n = stream.read(&mut buf[*read..]).await.map_err(err!())?;
                    *read += n;
                    if *read == buf.len() {
                        let msg = bincode::deserialize(buf).map_err(err!())?;
                        self.state = State::new();
                        debug!("receive {:?}", msg);
                        return Ok(Some(msg));
                    } else if n == 0 {
                        return Err(io::Error::from(ErrorKind::UnexpectedEof)).map_err(err!());
                    }
                }
            }
        }
    }
}
