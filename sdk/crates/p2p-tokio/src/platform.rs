//! Default platform implementation using standard library.

use std::net::UdpSocket;

use p2p_io::traits::Platform;

pub struct StdPlatform;

impl StdPlatform {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StdPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for StdPlatform {
    fn get_local_addresses(&self) -> Vec<String> {
        let mut addrs = Vec::new();

        // IPv4: connect UDP to public address to discover default-route IPv4
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
            if socket.connect("8.8.8.8:80").is_ok() {
                if let Ok(local) = socket.local_addr() {
                    let ip = local.ip().to_string();
                    if !ip.starts_with("0.") && ip != "0.0.0.0" {
                        addrs.push(ip);
                    }
                }
            }
        }

        // IPv6: connect UDP to public IPv6 address to discover default-route IPv6
        if let Ok(socket) = UdpSocket::bind("[::]:0") {
            if socket.connect("[2001:4860:4860::8888]:80").is_ok() {
                if let Ok(local) = socket.local_addr() {
                    let ip = local.ip().to_string();
                    // Filter out loopback and unspecified
                    if ip != "::1" && ip != "::" && ip != "0.0.0.0" {
                        addrs.push(ip);
                    }
                }
            }
        }

        if addrs.is_empty() {
            addrs.push("127.0.0.1".into());
        }

        addrs
    }

    fn random_bytes(&self, len: usize) -> Vec<u8> {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        (0..len).map(|_| rng.gen()).collect()
    }

    fn log(&self, tag: &str, msg: &str) {
        log::debug!("[{tag}] {msg}");
    }
}
