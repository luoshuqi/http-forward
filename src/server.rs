use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{debug, error, info, warn};
use md5::{Digest, Md5};
use rand::random;
use structopt::StructOpt;
use tokio::io::{copy_bidirectional, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{sleep, Duration};
use tokio_rustls::rustls::server::AllowAnyAuthenticatedClient;
use tokio_rustls::rustls::{RootCertStore, ServerConfig};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

use crate::http::{parse_domain, BAD_GATEWAY, GATEWAY_TIMEOUT};
use crate::protocol::{Protocol, Receiver, Request};
use crate::shared::Shared;
use crate::util::{init_logger, load_certs, load_key};
use crate::WithContext;

#[derive(Debug, StructOpt)]
struct Opt {
    /// http 绑定地址，格式为 "ip:端口"
    #[structopt(long)]
    http_addr: SocketAddr,

    /// http 证书 key
    #[structopt(long)]
    http_key: String,

    /// http 证书
    #[structopt(long)]
    http_cert: String,

    /// 绑定地址，格式为 "ip:端口"
    #[structopt(long)]
    addr: SocketAddr,

    /// 服务端证书 key
    #[structopt(long)]
    server_key: String,

    /// 服务端证书
    #[structopt(long)]
    server_cert: String,
}

pub async fn run() -> crate::Result<()> {
    init_logger();
    let opt: Opt = Opt::from_args();

    let http_acceptor = create_http_acceptor(&opt.http_key, &opt.http_cert)?;
    let http_listener = TcpListener::bind(opt.http_addr)
        .await
        .map_err(err!("cannot bind {}", opt.http_addr))?;
    let client_acceptor = create_client_acceptor(&opt.server_key, &opt.server_cert)?;
    let client_listener = TcpListener::bind(opt.addr)
        .await
        .map_err(err!("cannot bind {}", opt.addr))?;
    info!(
        "server started at {} {}",
        http_listener.local_addr().map_err(err!())?,
        client_listener.local_addr().map_err(err!())?
    );

    let mut sig_int = signal(SignalKind::interrupt()).map_err(err!())?;
    let mut sig_term = signal(SignalKind::terminate()).map_err(err!())?;
    let shared = Shared::new();
    loop {
        tokio::select! {
            accept = client_listener.accept() => {
                handle_client_accept(accept, &client_acceptor, &shared).await;
            }
            accept = http_listener.accept() => {
                handle_http_accept(accept, &http_acceptor, &shared).await;
            }
            _ = sig_int.recv() => {
                info!("catch SIGINT, exiting");
                break;
            }
            _ = sig_term.recv() => {
                info!("catch SIGTERM, exiting");
                break;
            }
        }
    }
    Ok(())
}

async fn handle_client_accept(
    accept: io::Result<(TcpStream, SocketAddr)>,
    acceptor: &Arc<TlsAcceptor>,
    shared: &Shared,
) {
    match accept {
        Ok((stream, addr)) => {
            debug!("client connection from {}", addr);
            let acceptor = Arc::clone(acceptor);
            let shared = shared.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, addr, acceptor, shared).await {
                    error!("{}", e);
                }
            });
        }
        Err(err) => error!("client accept error: {}", err),
    }
}

async fn handle_client(
    stream: TcpStream,
    addr: SocketAddr,
    acceptor: Arc<TlsAcceptor>,
    shared: Shared,
) -> crate::Result<()> {
    let mut stream = acceptor
        .accept(stream)
        .await
        .map_err(err!("Tls accept error"))
        .ctx("peer", addr)?;

    let mut receiver = Receiver::new();
    let msg = receiver.recv(&mut stream).await?;
    match msg {
        Some(Protocol::Register { domains })
            if !domains.is_empty() && !shared.client.exists(&domains) =>
        {
            Protocol::Ok.send(&mut stream).await.map_err(err!())?;
            let re = handle_register(stream, addr, &domains, &shared).await;
            shared.client.remove(&domains);
            re?
        }
        Some(Protocol::Register { .. }) => {
            Protocol::Error.send(&mut stream).await?;
            let _ = stream.shutdown().await;
        }
        Some(Protocol::Response { key }) => match shared.conn.remove(&key) {
            Some(sender) => match sender.send(stream) {
                Ok(()) => {}
                Err(mut stream) => {
                    let _ = stream.shutdown().await;
                }
            },
            None => {
                let _ = stream.shutdown().await;
            }
        },
        _ => {}
    }
    Ok(())
}

async fn handle_register(
    mut stream: TlsStream<TcpStream>,
    addr: SocketAddr,
    domains: &[String],
    shared: &Shared,
) -> crate::Result<()> {
    let mut tx = shared.client.add(domains.to_vec());
    let mut receiver = Receiver::new();
    loop {
        tokio::select! {
            msg = receiver.recv(&mut stream) => {
                match msg? {
                    Some(Protocol::Ping) => Protocol::Pong.send(&mut stream).await.map_err(err!())?,
                    Some(msg) => warn!("unexpected msg {:?} from {}", msg, addr),
                    None => break,
                }
            }
            msg = tx.recv() => {
                match msg {
                    Some(req) => Protocol::Request(req).send(&mut stream).await?,
                    None => {}
                }
            }
        }
    }

    let _ = stream.shutdown().await;
    Ok(())
}

async fn handle_http_accept(
    accept: io::Result<(TcpStream, SocketAddr)>,
    acceptor: &Arc<TlsAcceptor>,
    shared: &Shared,
) {
    match accept {
        Ok((stream, addr)) => {
            debug!("http connection from {}", addr);
            let acceptor = Arc::clone(acceptor);
            let shared = shared.clone();
            tokio::spawn(async move {
                match handle_http(stream, addr, acceptor, shared).await {
                    Ok(()) => {}
                    Err(err) => error!("{}", err),
                }
            });
        }
        Err(err) => error!("http accept error: {:?}", err),
    }
}

async fn handle_http(
    stream: TcpStream,
    addr: SocketAddr,
    acceptor: Arc<TlsAcceptor>,
    shared: Shared,
) -> crate::Result<()> {
    let mut stream = acceptor
        .accept(stream)
        .await
        .map_err(err!("Tls accept error"))
        .ctx("peer", addr)?;

    tokio::select! {
        result = parse_domain(&mut stream) => {
            let result = result?;
            if let Some(client) = shared.client.get(&result.domain) {
                let key = make_key(&result.domain);
                let req = Request::new(key.clone(), result.domain.clone());
                client.send(req).map_err(err!())?;
                let receiver = shared.conn.add(key.clone());

                tokio::select! {
                    conn = receiver => {
                        let mut conn = conn.map_err(err!())?;
                        conn.write_all(&result.buf).await.map_err(err!())?;
                        debug!("forward {} start", &result.domain);
                        copy_bidirectional(&mut stream, &mut conn).await.map_err(err!("forward {}", &result.domain))?;
                        debug!("forward {} end", &result.domain);
                    }
                    _ = sleep(Duration::from_secs(15)) => {
                        error!("{} timeout", result.domain);
                        GATEWAY_TIMEOUT.send(&mut stream).await?;
                        shared.conn.remove(&key);
                        let _ = stream.shutdown().await;
                    }
                }
            } else {
                error!("no client found for {}", result.domain);
                BAD_GATEWAY.send(&mut stream).await?;
                let _ = stream.shutdown().await;
            }
        }
        _ = sleep(Duration::from_secs(30)) => {
            let _ = stream.shutdown().await;
            error!("{} parse domain timeout", addr);
        }
    }

    Ok(())
}

fn make_key(domain: &str) -> Vec<u8> {
    let mut md5 = Md5::new();
    md5.update(domain.as_bytes());
    if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
        md5.update(d.as_secs().to_be_bytes());
    }
    let r: u64 = random();
    md5.update(r.to_be_bytes());
    md5.finalize().to_vec()
}

fn create_client_acceptor(key: &str, cert: &str) -> crate::Result<Arc<TlsAcceptor>> {
    let key = load_key(key)?;
    let cert = load_certs(cert)?;

    //把服务端证书加入 root，以信任由服务端证书签发的客户端证书
    let mut root = RootCertStore::empty();
    root.add(&cert[0]).map_err(err!())?;

    let verifier = AllowAnyAuthenticatedClient::new(root);
    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_client_cert_verifier(verifier)
        .with_single_cert(cert, key)
        .map_err(err!())?;
    Ok(Arc::new(TlsAcceptor::from(Arc::new(config))))
}

fn create_http_acceptor(key: &str, cert: &str) -> crate::Result<Arc<TlsAcceptor>> {
    let key = load_key(key)?;
    let cert = load_certs(cert)?;

    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert, key)
        .map_err(err!())?;
    Ok(Arc::new(TlsAcceptor::from(Arc::new(config))))
}
