use std::env::{set_var, var};
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};

use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
use tokio_rustls::rustls::{Certificate, PrivateKey};

// 读取证书
pub fn load_certs(path: &str) -> crate::Result<Vec<Certificate>> {
    let file = File::open(path).map_err(err!("cannot open {}", path))?;
    let certs = certs(&mut BufReader::new(file)).map_err(err!())?;
    assert!(!certs.is_empty(), "no cert found in {}", path);
    Ok(certs.into_iter().map(Certificate).collect::<Vec<_>>())
}

// 读取证书 key
pub fn load_key(path: &str) -> crate::Result<PrivateKey> {
    let file = File::open(path).map_err(err!("cannot open {}", path))?;
    let mut reader = BufReader::new(file);
    let mut keys = rsa_private_keys(&mut reader).map_err(err!())?;
    if keys.is_empty() {
        reader.seek(SeekFrom::Start(0)).map_err(err!())?;
        keys = pkcs8_private_keys(&mut reader).map_err(err!())?;
    }
    assert!(!keys.is_empty(), "no key found in {}", path);
    Ok(PrivateKey(keys.pop().unwrap()))
}

pub fn init_logger() {
    if var("RUST_LOG").is_err() {
        #[cfg(debug_assertions)]
        set_var("RUST_LOG", "debug");
        #[cfg(not(debug_assertions))]
        set_var("RUST_LOG", "info");
    }
    env_logger::init();
}
