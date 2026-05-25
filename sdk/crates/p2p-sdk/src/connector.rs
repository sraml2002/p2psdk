//! WebSocket signaling client (Connector protocol).

use std::thread;
use std::time::Duration;

use p2p_core::types::{
    ConnectorMessage, CONNECTOR_TYPE_REGISTER, CONNECTOR_TYPE_SEND,
    CONNECTOR_TYPE_REGISTER_OK, CONNECTOR_TYPE_MESSAGE,
};
use p2p_io::traits::SignalingTransport;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ConnectorError {
    NotConnected,
    NotRegistered,
    SendFailed(String),
}

impl std::fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => write!(f, "not connected"),
            Self::NotRegistered => write!(f, "not registered"),
            Self::SendFailed(msg) => write!(f, "send failed: {msg}"),
        }
    }
}

// ---------------------------------------------------------------------------
// ConnectorClient
// ---------------------------------------------------------------------------

pub struct ConnectorClient {
    ws: Box<dyn SignalingTransport>,
    identifier: String,
    registered: bool,
}

impl ConnectorClient {
    pub fn new(ws: Box<dyn SignalingTransport>) -> Self {
        Self {
            ws,
            identifier: String::new(),
            registered: false,
        }
    }

    /// Connect and register with the signaling server.
    pub fn connect(
        &mut self,
        url: &str,
        identifier: &str,
        auth_token: &str,
    ) -> Result<(), ConnectorError> {
        self.ws.connect(url).map_err(|e| ConnectorError::SendFailed(e.to_string()))?;
        self.identifier = identifier.into();
        self.registered = false;

        // Send register message
        let msg = ConnectorMessage {
            msg_type: CONNECTOR_TYPE_REGISTER.into(),
            from: identifier.into(),
            to: String::new(),
            data: serde_json::Value::String(String::new()),
            auth: auth_token.into(),
            reason: String::new(),
            timestamp: 0,
        };
        let json = serde_json::to_string(&msg).map_err(|e| ConnectorError::SendFailed(e.to_string()))?;
        self.ws.send(&json).map_err(|e| ConnectorError::SendFailed(e.to_string()))?;
        Ok(())
    }

    /// Poll for incoming messages. Call this in a loop.
    ///
    /// Returns parsed messages that need handling (MESSAGE type).
    /// Handles REGISTER_OK internally.
    pub fn poll(&mut self) -> Vec<ConnectorMessage> {
        let mut incoming = Vec::new();
        while let Some(text) = self.ws.try_recv() {
            if let Ok(msg) = serde_json::from_str::<ConnectorMessage>(&text) {
                match msg.msg_type.as_str() {
                    CONNECTOR_TYPE_REGISTER_OK => {
                        self.registered = true;
                        log::debug!("Connector registered OK");
                    }
                    CONNECTOR_TYPE_MESSAGE => {
                        incoming.push(msg);
                    }
                    _ => {
                        log::debug!("Connector msg type: {}", msg.msg_type);
                    }
                }
            }
        }
        incoming
    }

    /// Send a message to a target peer.
    pub fn send_to(&self, target_id: &str, data: &serde_json::Value) -> Result<(), ConnectorError> {
        if !self.ws.is_connected() {
            return Err(ConnectorError::NotConnected);
        }
        let msg = ConnectorMessage {
            msg_type: CONNECTOR_TYPE_SEND.into(),
            from: self.identifier.clone(),
            to: target_id.into(),
            data: data.clone(),
            auth: String::new(),
            reason: String::new(),
            timestamp: 0,
        };
        let json = serde_json::to_string(&msg).map_err(|e| ConnectorError::SendFailed(e.to_string()))?;
        self.ws.send(&json).map_err(|e| ConnectorError::SendFailed(e.to_string()))?;
        Ok(())
    }

    pub fn is_registered(&self) -> bool { self.registered }
    pub fn is_connected(&self) -> bool { self.ws.is_connected() }

    pub fn disconnect(&mut self) {
        self.ws.close();
        self.registered = false;
    }

    /// Blocking poll loop with automatic reconnect.
    /// Runs until `stop` becomes true. Calls `on_message` for each incoming MESSAGE.
    /// Calls `on_state_change` when registration status changes.
    /// On disconnect, retries with exponential backoff (1s → 30s).
    pub fn run_with_reconnect(
        &mut self,
        url: &str,
        identifier: &str,
        auth_token: &str,
        stop: &std::sync::atomic::AtomicBool,
        on_message: &dyn Fn(ConnectorMessage),
        on_state_change: &dyn Fn(bool),
    ) {
        use std::sync::atomic::Ordering;
        let initial_delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(30);
        let mut delay = initial_delay;

        while !stop.load(Ordering::Relaxed) {
            // Connect
            match self.connect(url, identifier, auth_token) {
                Ok(()) => {
                    delay = initial_delay;
                    log::info!("Connector connected to {}", url);
                }
                Err(e) => {
                    log::error!("Connector connect failed: {}", e);
                    thread::sleep(delay);
                    delay = (delay * 2).min(max_delay);
                    continue;
                }
            }

            // Poll loop
            while !stop.load(Ordering::Relaxed) {
                let was_registered = self.registered;
                let msgs = self.poll();

                if !self.ws.is_connected() {
                    log::warn!("Connector disconnected");
                    self.registered = false;
                    if was_registered {
                        on_state_change(false);
                    }
                    break;
                }

                if self.registered && !was_registered {
                    on_state_change(true);
                }

                for msg in msgs {
                    on_message(msg);
                }

                thread::sleep(Duration::from_millis(50));
            }

            // Reconnect delay
            if !stop.load(Ordering::Relaxed) {
                thread::sleep(delay);
                delay = (delay * 2).min(max_delay);
            }
        }

        self.disconnect();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connector_error_display() {
        assert_eq!(format!("{}", ConnectorError::NotConnected), "not connected");
    }

    #[test]
    fn test_connector_message_serialize() {
        let msg = ConnectorMessage {
            msg_type: "register".into(),
            from: "user1".into(),
            to: String::new(),
            data: serde_json::Value::Null,
            auth: "token123".into(),
            reason: String::new(),
            timestamp: 0,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"register\""));
        assert!(json.contains("\"from\":\"user1\""));
    }
}
