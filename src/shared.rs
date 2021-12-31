use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use tokio::net::TcpStream;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot::{self, Receiver, Sender};
use tokio_rustls::server::TlsStream;

use crate::protocol::Request;

// 共享状态
#[derive(Clone)]
pub struct Shared {
    pub client: ClientChannel,
    pub conn: ConnChannel,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            client: ClientChannel::new(),
            conn: ConnChannel::new(),
        }
    }
}

// 客户端集合, key 为域名, value 用来发送转发请求
#[derive(Clone)]
pub struct ClientChannel(Arc<RwLock<HashMap<String, UnboundedSender<Request>>>>);

impl ClientChannel {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }

    // 是否已存在处理 domains 中任意一个域名的客户端
    pub fn exists(&self, domains: &[String]) -> bool {
        let map = self.0.read().unwrap();
        for v in domains {
            if map.contains_key(v) {
                return true;
            }
        }
        false
    }

    pub fn get(&self, domain: &str) -> Option<UnboundedSender<Request>> {
        self.0.read().unwrap().get(domain).map(Clone::clone)
    }

    pub fn add(&self, domains: Vec<String>) -> UnboundedReceiver<Request> {
        let (tx, rx) = unbounded_channel();
        let mut map = self.0.write().unwrap();
        for d in domains {
            map.insert(d, tx.clone());
        }
        rx
    }

    pub fn remove(&self, domains: &[String]) {
        let mut map = self.0.write().unwrap();
        for d in domains {
            map.remove(d);
        }
    }
}

// 待转发连接集合, key 为标识, value 用来发送目标连接
#[derive(Clone)]
pub struct ConnChannel(Arc<Mutex<HashMap<Vec<u8>, Sender<TlsStream<TcpStream>>>>>);

impl ConnChannel {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }

    pub fn add(&self, key: Vec<u8>) -> Receiver<TlsStream<TcpStream>> {
        let (tx, rx) = oneshot::channel();
        self.0.lock().unwrap().insert(key, tx);
        rx
    }

    pub fn remove(&self, key: &[u8]) -> Option<Sender<TlsStream<TcpStream>>> {
        self.0.lock().unwrap().remove(key)
    }
}
