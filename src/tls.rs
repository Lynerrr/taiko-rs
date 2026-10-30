//! TLS for connections to the online server. Everything else reads a socket on one thread and
//! writes it on another, which a TLS stream can't do, so each connection is pumped between the
//! encrypted socket and a plain one on loopback, and what's handed on is that plain socket.
//!
//! The certificate is our own: the game carries the CA (assets/server-ca.der, its key is off
//! the server) and trusts nothing else, so there's nothing to renew and no public CA to trust.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, Connection, RootCertStore, ServerConfig, ServerConnection};

/// The port the game's connections take, encrypted (`online::DEFAULT_PORT` is the old plain one).
pub const PORT: u16 = 7801;
/// The server these are for. Others (one of your own, on a LAN) stay plain.
pub const SERVER_HOST: &str = "jashincraft.space";

const CA: &[u8] = include_bytes!("../assets/server-ca.der");
/// How long the handshake may take.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// How long a write may stall before the connection counts as gone.
const WRITE_STALL: Duration = Duration::from_secs(20);

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn client_config(ca: &[u8]) -> Result<Arc<ClientConfig>, String> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca.to_vec())).map_err(|e| e.to_string())?;
    let builder = ClientConfig::builder_with_provider(provider()).with_protocol_versions(&[&rustls::version::TLS13]).map_err(|e| e.to_string())?;
    Ok(Arc::new(builder.with_root_certificates(roots).with_no_client_auth()))
}

/// Encrypts a connection to the server, proving it's the server: the plain end, to use as the
/// connection. Failing here is never a reason to go on without.
pub fn connect(raw: TcpStream) -> Result<TcpStream, String> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    let config = CONFIG.get_or_init(|| client_config(CA)).clone()?;
    connect_with(raw, config, SERVER_HOST)
}

fn connect_with(mut raw: TcpStream, config: Arc<ClientConfig>, host: &str) -> Result<TcpStream, String> {
    let name = ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
    let mut conn = ClientConnection::new(config, name).map_err(|e| e.to_string())?;
    handshake(&mut raw, &mut conn)?;
    pump(raw, conn).map_err(|e| e.to_string())
}

fn handshake<D: rustls::SideData>(raw: &mut TcpStream, conn: &mut rustls::ConnectionCommon<D>) -> Result<(), String> {
    let _ = raw.set_read_timeout(Some(HANDSHAKE));
    let _ = raw.set_write_timeout(Some(HANDSHAKE));
    while conn.is_handshaking() {
        conn.complete_io(raw).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// What the server answers with: DIR's certificate and key.
#[allow(dead_code)]
pub fn server_config(cert: &Path, key: &Path) -> Result<Arc<ServerConfig>, String> {
    let read = |p: &Path| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    server_config_of(read(cert)?, read(key)?)
}

#[allow(dead_code)]
fn server_config_of(cert: Vec<u8>, key: Vec<u8>) -> Result<Arc<ServerConfig>, String> {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key));
    let builder = ServerConfig::builder_with_provider(provider()).with_protocol_versions(&[&rustls::version::TLS13]).map_err(|e| e.to_string())?;
    let config = builder.with_no_client_auth().with_single_cert(vec![CertificateDer::from(cert)], key).map_err(|e| e.to_string())?;
    Ok(Arc::new(config))
}

/// The server's side of `connect`.
#[allow(dead_code)]
pub fn accept(mut raw: TcpStream, config: Arc<ServerConfig>) -> Result<TcpStream, String> {
    let mut conn = ServerConnection::new(config).map_err(|e| e.to_string())?;
    handshake(&mut raw, &mut conn)?;
    pump(raw, conn).map_err(|e| e.to_string())
}

/// A connected pair on loopback. The port is open for a moment, so whoever connected is checked.
fn loopback() -> io::Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let near = TcpStream::connect(listener.local_addr()?)?;
    let (far, peer) = listener.accept()?;
    if peer != near.local_addr()? {
        return Err(io::Error::new(ErrorKind::PermissionDenied, "someone else connected"));
    }
    near.set_nodelay(true)?;
    far.set_nodelay(true)?;
    Ok((near, far))
}

/// Starts moving data between `raw` and a new plain socket, which it returns.
fn pump(raw: TcpStream, conn: impl Into<Connection>) -> io::Result<TcpStream> {
    let (plain, inner) = loopback()?;
    raw.set_read_timeout(None)?;
    raw.set_write_timeout(Some(WRITE_STALL))?;
    let _ = raw.set_nodelay(true);
    let conn = Arc::new(Mutex::new(conn.into()));
    let turn = Arc::new(Mutex::new(()));
    let (raw_up, inner_up, conn_up, turn_up) = (raw.try_clone()?, inner.try_clone()?, conn.clone(), turn.clone());
    std::thread::Builder::new().spawn(move || {
        from_network(&raw_up, &inner_up, &conn_up, &turn_up);
        let _ = raw_up.shutdown(Shutdown::Both);
        let _ = inner_up.shutdown(Shutdown::Both);
    })?;
    std::thread::Builder::new().spawn(move || {
        to_network(&raw, &inner, &conn, &turn);
        let _ = raw.shutdown(Shutdown::Both);
        let _ = inner.shutdown(Shutdown::Both);
    })?;
    Ok(plain)
}

/// Sends what `conn` has for the network, with the turn to send (records can't change places).
/// The socket is never written with `conn` locked: both ends stuck writing, each with a full
/// buffer the other can't read because reading needs that lock, would wait for each other.
fn send_pending<'a>(turn: MutexGuard<()>, mut locked: MutexGuard<'a, Connection>, conn: &'a Mutex<Connection>, mut raw: &TcpStream) -> io::Result<()> {
    loop {
        let mut out = Vec::new();
        while locked.wants_write() {
            locked.write_tls(&mut out)?;
        }
        if out.is_empty() {
            // The turn goes first: whoever adds more once `conn` is free then gets it.
            drop(turn);
            return Ok(());
        }
        drop(locked);
        raw.write_all(&out)?;
        locked = conn.lock().unwrap();
    }
}

/// `send_pending` from the reading side, which mustn't wait to send or it stops reading: if
/// someone has the turn, they send this too.
fn answer<'a>(locked: MutexGuard<'a, Connection>, conn: &'a Mutex<Connection>, turn: &Mutex<()>, raw: &TcpStream) -> io::Result<()> {
    match turn.try_lock() {
        Ok(turn) => send_pending(turn, locked, conn, raw),
        Err(_) => Ok(()),
    }
}

/// Decrypts what comes in, until either side is done.
fn from_network(mut raw: &TcpStream, mut inner: &TcpStream, conn: &Mutex<Connection>, turn: &Mutex<()>) {
    let mut buf = [0u8; 16 * 1024];
    let mut plain = [0u8; 16 * 1024];
    loop {
        let n = match raw.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let mut rest = &buf[..n];
        let mut out = Vec::new();
        let mut closed = false;
        {
            let mut locked = conn.lock().unwrap();
            while !rest.is_empty() {
                if locked.read_tls(&mut rest).is_err() {
                    return;
                }
                if locked.process_new_packets().is_err() {
                    let _ = answer(locked, conn, turn, raw);
                    return;
                }
                loop {
                    match locked.reader().read(&mut plain) {
                        Ok(0) => {
                            closed = true;
                            break;
                        }
                        Ok(n) => out.extend_from_slice(&plain[..n]),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => return,
                    }
                }
            }
            if answer(locked, conn, turn, raw).is_err() {
                return;
            }
        }
        // Outside the lock: a slow reader here mustn't hold up the other direction.
        if inner.write_all(&out).is_err() || closed {
            return;
        }
    }
}

/// Encrypts what goes out, until either side is done.
fn to_network(raw: &TcpStream, mut inner: &TcpStream, conn: &Mutex<Connection>, turn: &Mutex<()>) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = match inner.read(&mut buf) {
            Ok(n) => n,
            Err(_) => 0,
        };
        let my_turn = turn.lock().unwrap();
        let mut locked = conn.lock().unwrap();
        if n == 0 {
            locked.send_close_notify();
            let _ = send_pending(my_turn, locked, conn, raw);
            return;
        }
        if locked.writer().write_all(&buf[..n]).is_err() || send_pending(my_turn, locked, conn, raw).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    // A CA and certificate made for these tests alone: they open nothing.
    const TEST_CA: &[u8] = include_bytes!("tls_test/ca.der");
    const TEST_CERT: &[u8] = include_bytes!("tls_test/cert.der");
    const TEST_KEY: &[u8] = include_bytes!("tls_test/key.der");
    const HOST: &str = "test.invalid";

    /// A server that answers each line with the same line, and the socket to reach it.
    fn echo_server() -> TcpStream {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let config = server_config_of(TEST_CERT.to_vec(), TEST_KEY.to_vec()).unwrap();
        std::thread::spawn(move || {
            let (raw, _) = listener.accept().unwrap();
            let Ok(plain) = accept(raw, config) else { return };
            let mut writer = plain.try_clone().unwrap();
            for line in BufReader::new(plain).lines() {
                let line = line.unwrap();
                writer.write_all(format!("{line}\n").as_bytes()).unwrap();
            }
        });
        client
    }

    #[test]
    fn lines_go_both_ways_at_once_on_a_cloned_socket() {
        let plain = connect_with(echo_server(), client_config(TEST_CA).unwrap(), HOST).unwrap();
        plain.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut writer = plain.try_clone().unwrap();
        let line = "x".repeat(200_000);
        let sent = line.clone();
        let sending = std::thread::spawn(move || {
            for _ in 0..50 {
                writer.write_all(format!("{sent}\n").as_bytes()).unwrap();
            }
        });
        let mut reader = BufReader::new(plain);
        for _ in 0..50 {
            let mut got = String::new();
            reader.read_line(&mut got).unwrap();
            assert_eq!(got.trim_end().len(), line.len());
        }
        sending.join().unwrap();
    }

    #[test]
    fn closing_the_plain_side_closes_the_other() {
        let plain = connect_with(echo_server(), client_config(TEST_CA).unwrap(), HOST).unwrap();
        plain.shutdown(Shutdown::Both).unwrap();
    }

    #[test]
    fn a_server_without_our_certificate_is_refused() {
        // It has a certificate, but for another name, and then from another CA.
        assert!(connect_with(echo_server(), client_config(TEST_CA).unwrap(), "other.invalid").is_err());
        assert!(connect_with(echo_server(), client_config(CA).unwrap(), HOST).is_err());
    }

    #[test]
    fn the_real_ca_loads() {
        assert!(client_config(CA).is_ok());
    }

    #[test]
    fn a_plain_client_gets_nothing_from_the_encrypted_port() {
        let mut raw = echo_server();
        raw.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        raw.write_all(b"{\"protocol\":1}\n").unwrap();
        let mut got = Vec::new();
        let _ = raw.read_to_end(&mut got);
        assert!(!got.windows(8).any(|w| w == b"protocol"));
    }
}
