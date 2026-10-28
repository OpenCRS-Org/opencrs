//! A local HTTPS server with a throwaway certificate for `localhost`, for
//! tests: the real client (rustls, the redirect rules, the address guard) run
//! against a real TLS handshake, with no network. Like `server::Server` but
//! over TLS; every response closes the connection.

use crate::server::{Req, Resp};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use ureq::tls::{Certificate, RootCerts};

pub struct TlsServer {
    pub port: u16,
    /// The paths asked for, in order.
    pub seen: Arc<Mutex<Vec<String>>>,
    cert_pem: String,
}

impl TlsServer {
    /// Starts serving; the handler gets the request and the server's port.
    /// The server lives until the test process ends.
    pub fn start(handler: impl Fn(&Req, u16) -> Resp + Send + Sync + 'static) -> TlsServer {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], key.into())
        .unwrap();
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (handler, seen2) = (Arc::new(handler), seen.clone());
        std::thread::spawn(move || {
            for tcp in listener.incoming() {
                let Ok(tcp) = tcp else { continue };
                let (config, handler, seen) = (config.clone(), handler.clone(), seen2.clone());
                std::thread::spawn(move || {
                    let conn = rustls::ServerConnection::new(config).unwrap();
                    let mut tls = rustls::StreamOwned::new(conn, tcp);
                    // The request: up to the blank line after its headers.
                    let mut head = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        match tls.read(&mut buf) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => head.extend_from_slice(&buf[..n]),
                        }
                    }
                    let text = String::from_utf8_lossy(&head);
                    let path = text
                        .lines()
                        .next()
                        .and_then(|l| l.split(' ').nth(1))
                        .unwrap_or("/")
                        .to_string();
                    let authorization = text
                        .lines()
                        .find_map(|l| l.strip_prefix("authorization: "))
                        .unwrap_or("")
                        .to_string();
                    seen.lock().unwrap().push(path.clone());
                    let resp = handler(
                        &Req {
                            path,
                            authorization,
                        },
                        port,
                    );
                    let mut out = format!("HTTP/1.1 {} X\r\n", resp.status);
                    for (k, v) in &resp.headers {
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    out.push_str(&format!(
                        "Content-Length: {}\r\nConnection: close\r\n\r\n",
                        resp.body.len()
                    ));
                    let _ = tls.write_all(out.as_bytes());
                    let _ = tls.write_all(&resp.body);
                    tls.conn.send_close_notify();
                    let _ = tls.flush();
                });
            }
        });
        TlsServer {
            port,
            seen,
            cert_pem: cert.cert.pem(),
        }
    }

    pub fn url(&self) -> String {
        format!("https://localhost:{}", self.port)
    }

    /// The roots a client must trust to talk to this server.
    pub fn roots(&self) -> RootCerts {
        RootCerts::new_with_certs(&[Certificate::from_pem(self.cert_pem.as_bytes()).unwrap()])
    }
}
