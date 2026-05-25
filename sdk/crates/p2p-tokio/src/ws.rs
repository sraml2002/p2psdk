//! Synchronous WebSocket signaling transport using tungstenite.

use std::net::TcpStream;
use std::sync::Mutex;

use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message};

type WsStream = tungstenite::protocol::WebSocket<MaybeTlsStream<TcpStream>>;

use p2p_io::traits::{IoError, SignalingTransport};

/// Set the underlying TCP stream to non-blocking mode.
fn set_nonblocking(ws: &mut WsStream, nonblocking: bool) -> std::io::Result<()> {
    let stream = ws.get_mut();
    match stream {
        MaybeTlsStream::Plain(tcp) => tcp.set_nonblocking(nonblocking),
        MaybeTlsStream::Rustls(tls) => tls.get_mut().set_nonblocking(nonblocking),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "cannot set nonblocking on this stream type",
        )),
    }
}

pub struct SyncSignalingTransport {
    socket: Mutex<Option<WsStream>>,
}

impl SyncSignalingTransport {
    pub fn new() -> Self {
        Self {
            socket: Mutex::new(None),
        }
    }
}

impl Default for SyncSignalingTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl SignalingTransport for SyncSignalingTransport {
    fn connect(&mut self, url: &str) -> Result<(), IoError> {
        let (ws_socket, _response) = connect(url)
            .map_err(|e| IoError::Other(format!("WS connect failed: {e}")))?;
        *self.socket.lock().unwrap() = Some(ws_socket);
        Ok(())
    }

    fn send(&self, message: &str) -> Result<(), IoError> {
        let mut guard = self.socket.lock().unwrap();
        let ws = guard.as_mut().ok_or(IoError::Closed)?;
        ws.send(Message::Text(message.into()))
            .map_err(|e| IoError::Other(format!("WS send failed: {e}")))?;
        Ok(())
    }

    fn try_recv(&mut self) -> Option<String> {
        let mut guard = self.socket.lock().unwrap();
        let ws = guard.as_mut()?;

        // Set non-blocking, try one read, then restore blocking mode
        if set_nonblocking(ws, true).is_err() {
            return None;
        }
        let result = ws.read();
        let _ = set_nonblocking(ws, false);

        match result {
            Ok(Message::Text(text)) => Some(text.into()),
            Ok(Message::Binary(data)) => Some(String::from_utf8_lossy(&data).into_owned()),
            Ok(_) => None,
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                None
            }
            Err(_) => None,
        }
    }

    fn close(&mut self) {
        if let Some(mut ws) = self.socket.lock().unwrap().take() {
            let _ = ws.close(None);
        }
    }

    fn is_connected(&self) -> bool {
        self.socket.lock().unwrap().is_some()
    }
}
