//! Synchronous UDP transport using std::net::UdpSocket.

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use p2p_io::traits::{IoError, UdpTransport};

pub struct SyncUdpTransport {
    socket: UdpSocket,
}

impl SyncUdpTransport {
    pub fn bind(addr: &str, port: u16) -> Result<Self, IoError> {
        let addr: SocketAddr = format!("{addr}:{port}")
            .parse()
            .map_err(|e| IoError::Other(format!("invalid address: {e}")))?;
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(false)?;
        Ok(Self { socket })
    }

    pub fn bind_any(port: u16) -> Result<Self, IoError> {
        Self::bind("0.0.0.0", port)
    }

    pub fn bind_any_v6(port: u16) -> Result<Self, IoError> {
        Self::bind("[::]", port)
    }
}

impl UdpTransport for SyncUdpTransport {
    fn send_to(&self, data: &[u8], addr: &str, port: u16) -> Result<(), IoError> {
        let target: SocketAddr = format!("{addr}:{port}")
            .parse()
            .map_err(|e| IoError::Other(format!("invalid target: {e}")))?;
        self.socket.send_to(data, target)?;
        Ok(())
    }

    fn recv_from(&self, timeout_ms: u64) -> Result<(Vec<u8>, String, u16), IoError> {
        self.socket
            .set_read_timeout(Some(Duration::from_millis(timeout_ms)))
            .map_err(|e| IoError::Other(format!("set_read_timeout: {e}")))?;

        let mut buf = [0u8; 65535];
        match self.socket.recv_from(&mut buf) {
            Ok((len, addr)) => {
                let ip = addr.ip().to_string();
                let port = addr.port();
                Ok((buf[..len].to_vec(), ip, port))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                Err(IoError::Timeout)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn local_addr(&self) -> Result<(String, u16), IoError> {
        let addr = self.socket.local_addr()?;
        Ok((addr.ip().to_string(), addr.port()))
    }

    fn close(&self) {
        // UdpSocket closes on drop; nothing explicit needed
    }
}
