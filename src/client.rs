use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;

use log::{debug, error, info};
use structopt::StructOpt;
use tokio::io::{copy_bidirectional, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{sleep, Duration};
use tokio_rustls::rustls::{ClientConfig, RootCertStore, ServerName};
use tokio_rustls::TlsConnector;

use crate::protocol::{Protocol, Receiver, Request};
use crate::util::{init_logger, load_certs, load_key};

// 命令行参数
#[derive(Debug, StructOpt)]
struct Opt {
    /// 服务器地址, 格式为"域名:端口"
    #[structopt(short, long)]
    server_addr: String,

    /// 转发配置，格式为"域名:转发地址"。示例："a.foo.com:127.0.0.1:80" 表示把对 a.foo.com 的请求转发到127.0.0.1:80
    #[structopt(short, long)]
    forward: Vec<ForwardOption>,

    /// 客户端证书 key
    #[structopt(short = "k", long)]
    client_key: String,

    /// 客户端证书
    #[structopt(short, long)]
    client_cert: String,
}

pub async fn run() -> crate::Result<()> {
    init_logger();
    let opt = validate_opt();

    let mut sig_int = signal(SignalKind::interrupt()).map_err(err!())?;
    let mut sig_term = signal(SignalKind::terminate()).map_err(err!())?;

    let server_name = ServerName::try_from(opt.server_addr.split(':').next().unwrap()).unwrap();
    let connector = create_connector(&opt)?;
    let server_stream = TcpStream::connect(&opt.server_addr)
        .await
        .map_err(err!("cannot connect to {}", opt.server_addr))?;
    let mut server_stream = connector
        .connect(server_name.clone(), server_stream)
        .await
        .map_err(err!("cannot connect to {}", opt.server_addr))?;

    let mut forward = HashMap::new();
    let mut domains = Vec::with_capacity(opt.forward.len());
    for v in opt.forward {
        domains.push(v.domain.clone());
        forward.insert(v.domain, v.destination);
    }
    let msg = Protocol::Register { domains };
    msg.send(&mut server_stream).await.map_err(err!())?;

    let mut receiver = Receiver::new();
    loop {
        tokio::select! {
            msg = receiver.recv(&mut server_stream) => {
                match msg? {
                    Some(Protocol::Ok) => {
                        info!("register ok");
                    }
                    Some(Protocol::Error) => {
                        error!("register error");
                        let _ = server_stream.shutdown().await;
                        exit(1);
                    }
                    Some(Protocol::Pong) => {}
                    Some(Protocol::Request(req)) => {
                        let dst = forward.get(&req.domain).unwrap().clone();
                        let server_name = server_name.clone();
                        let server_addr = opt.server_addr.clone();
                        let connector = connector.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_forward(req, dst, server_addr, server_name, connector).await {
                                error!("{}", e);
                            }
                        });
                    }
                    Some(_) => {}
                    None => {
                        info!("server closed");
                        break;
                    }
                }
            }
            _ = sleep(Duration::from_secs(60)) => {
                Protocol::Ping.send(&mut server_stream).await.map_err(err!())?;
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

    let _ = server_stream.shutdown().await;
    Ok(())
}

async fn handle_forward(
    req: Request,
    destination: String,
    server_addr: String,
    server_name: ServerName,
    connector: TlsConnector,
) -> crate::Result<()> {
    let mut dst_stream = TcpStream::connect(&destination)
        .await
        .map_err(err!("cannot connect to {}", destination))?;
    let server_stream = TcpStream::connect(&server_addr)
        .await
        .map_err(err!("cannot connect to {}", server_addr))?;
    let mut server_stream = connector
        .connect(server_name, server_stream)
        .await
        .map_err(err!("cannot connect to {}", server_addr))?;

    Protocol::Response { key: req.key }
        .send(&mut server_stream)
        .await
        .map_err(err!())?;

    debug!("{} <=> {}", &req.domain, destination);
    copy_bidirectional(&mut server_stream, &mut dst_stream)
        .await
        .map_err(err!("{} <=> {}", &req.domain, destination))?;
    Ok(())
}

fn create_connector(opt: &Opt) -> crate::Result<TlsConnector> {
    let key = load_key(&opt.client_key)?;
    let cert = load_certs(&opt.client_cert)?;

    //把服务端证书加入 root，以信任服务端证书
    let mut root = RootCertStore::empty();
    for v in cert.iter().skip(1) {
        root.add(v).map_err(err!())?;
    }

    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root)
        .with_single_cert(cert, key)
        .map_err(err!())?;

    Ok(TlsConnector::from(Arc::new(config)))
}

fn validate_opt() -> Opt {
    let opt: Opt = Opt::from_args();
    if opt.forward.is_empty() {
        eprintln!("missing --forward <forward>");
        exit(1);
    }

    match opt.server_addr.split(':').next() {
        Some(v) => match ServerName::try_from(v) {
            Ok(_) => {}
            Err(_) => {
                eprintln!("{}: Wrong format", opt.server_addr);
                exit(1)
            }
        },
        None => {
            eprintln!("{}: Wrong format", opt.server_addr);
            exit(1);
        }
    }
    opt
}

// 转发配置
#[derive(Debug)]
struct ForwardOption {
    domain: String,      // 域名
    destination: String, // 目的地址
}

#[derive(Debug)]
struct InvalidForwardOption;

impl Display for InvalidForwardOption {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt("wrong format", f)
    }
}

impl FromStr for ForwardOption {
    type Err = InvalidForwardOption;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.find(':') {
            Some(n) if n < s.len() - 1 => Ok(ForwardOption {
                domain: s[..n].to_string(),
                destination: s[n + 1..].to_string(),
            }),
            _ => Err(InvalidForwardOption),
        }
    }
}
