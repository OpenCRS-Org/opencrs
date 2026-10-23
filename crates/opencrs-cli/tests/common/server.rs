//! A tiny HTTP/1.1 server on a local port, for fetch tests. One thread per
//! connection; every response closes the connection.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub struct Req {
    pub path: String,
    pub authorization: String,
}

pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn ok(body: impl Into<Vec<u8>>) -> Resp {
        Resp {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
        }
    }
    pub fn status(code: u16) -> Resp {
        Resp {
            status: code,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
    pub fn redirect(to: &str) -> Resp {
        Resp::status(302).header("Location", to)
    }
    pub fn header(mut self, k: &str, v: &str) -> Resp {
        self.headers.push((k.into(), v.into()));
        self
    }
}

pub struct Server {
    pub addr: String,
    pub seen: Arc<Mutex<Vec<(String, String)>>>,
}

impl Server {
    /// Starts serving; the handler gets the port-less request. The server
    /// lives until the test process ends.
    pub fn start(handler: impl Fn(&Req, &str) -> Resp + Send + Sync + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let (seen2, addr2) = (seen.clone(), addr.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (handler, seen, addr) = (handler.clone(), seen2.clone(), addr2.clone());
                std::thread::spawn(move || {
                    let mut r = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                    let mut auth = String::new();
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                        if h.to_ascii_lowercase().starts_with("authorization:") {
                            auth = h["authorization:".len()..].trim().to_string();
                        }
                    }
                    seen.lock().unwrap().push((path.clone(), auth.clone()));
                    let resp = handler(
                        &Req {
                            path,
                            authorization: auth,
                        },
                        &addr,
                    );
                    let mut out = format!("HTTP/1.1 {} X\r\n", resp.status);
                    for (k, v) in &resp.headers {
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    out.push_str(&format!(
                        "Content-Length: {}\r\nConnection: close\r\n\r\n",
                        resp.body.len()
                    ));
                    let _ = stream.write_all(out.as_bytes());
                    let _ = stream.write_all(&resp.body);
                });
            }
        });
        Server { addr, seen }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}
