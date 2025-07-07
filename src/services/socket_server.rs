use log::{info, error, warn, debug};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};

use crate::config::Config;
use crate::devices::traits::DeviceData;
use crate::utils::error::ModbusError;

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub address: String,
    pub connected_at: chrono::DateTime<chrono::Utc>,
    pub bytes_sent: u64,
    pub messages_sent: u64,
}

pub struct WebSocketServer {
    config: Config,
    clients: Arc<RwLock<HashMap<String, tokio_tungstenite::WebSocketStream<TcpStream>>>>,
    client_stats: Arc<RwLock<HashMap<String, ClientInfo>>>,
    is_running: Arc<RwLock<bool>>,
    port: u16,
}

impl WebSocketServer {
    pub fn new(config: Config) -> Self {
        let port = config.socket_server.port;
        Self {
            config,
            clients: Arc::new(RwLock::new(HashMap::new())),
            client_stats: Arc::new(RwLock::new(HashMap::new())),
            is_running: Arc::new(RwLock::new(false)),
            port,
        }
    }

    pub async fn start(&self) -> Result<(), ModbusError> {
        let bind_address = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&bind_address).await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to bind WebSocket server: {}", e))
        })?;

        info!("🔌 WebSocket server listening on {}", bind_address);
        *self.is_running.write().await = true;

        while *self.is_running.read().await {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("🔗 New client attempting WebSocket connection: {}", addr);
                    let client_id = addr.to_string();

                    match accept_async(stream).await {
                        Ok(ws_stream) => {
                            info!("✅ WebSocket connection established with {}", client_id);

                            let client_info = ClientInfo {
                                address: client_id.clone(),
                                connected_at: chrono::Utc::now(),
                                bytes_sent: 0,
                                messages_sent: 0,
                            };

                            self.clients.write().await.insert(client_id.clone(), ws_stream);
                            self.client_stats.write().await.insert(client_id, client_info);
                        }
                        Err(e) => {
                            warn!("❌ Failed to establish WebSocket connection with {}: {}", client_id, e);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Send a message to all clients using a WebSocket endpoint
    pub async fn send_device_data(&self, device_data: &dyn DeviceData) -> Result<(), ModbusError> {
        let message = serde_json::json!({
            "endpoint": "/flowmeter/read",
            "type": "flowmeter_data",
            "timestamp": device_data.unix_ts(),
            "data": device_data.get_parameters_as_floats()
        });

        self.broadcast_message(&message).await?;
        Ok(())
    }

    async fn broadcast_message(&self, message: &Value) -> Result<(), ModbusError> {
        let message_text = serde_json::to_string(message).map_err(|e| {
            ModbusError::InvalidData(format!("Failed to serialize message: {}", e))
        })?;

        let mut clients = self.clients.write().await;
        let mut client_stats = self.client_stats.write().await;
        let mut clients_to_remove = Vec::new();

        for (client_id, ws_stream) in clients.iter_mut() {
            let ws_message = Message::Text(message_text.clone());
            match timeout(Duration::from_secs(5), ws_stream.send(ws_message)).await {
                Ok(Ok(())) => {
                    if let Some(stats) = client_stats.get_mut(client_id) {
                        stats.bytes_sent += message_text.len() as u64;
                        stats.messages_sent += 1;
                    }
                }
                Ok(Err(e)) => {
                    warn!("Failed to send WebSocket message to client {}: {}", client_id, e);
                    clients_to_remove.push(client_id.clone());
                }
                Err(_) => {
                    warn!("Timeout sending WebSocket message to client {}", client_id);
                    clients_to_remove.push(client_id.clone());
                }
            }
        }

        for client_id in clients_to_remove {
            clients.remove(&client_id);
            client_stats.remove(&client_id);
            info!("🔌 WebSocket client {} disconnected", client_id);
        }

        Ok(())
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}