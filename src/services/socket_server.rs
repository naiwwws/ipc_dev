use log::{info, warn, error, debug};
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, RwLock};
use tokio::io::AsyncWriteExt;
use chrono::{DateTime, Utc};

use crate::devices::DeviceData;
use crate::utils::error::ModbusError;


#[derive(Debug, Clone, serde::Serialize)]
pub struct SocketMessage {
    pub timestamp: DateTime<Utc>,
    pub device_address: u8,
    pub device_name: String,
    pub device_uuid: String,
    pub data: HashMap<String, String>,
}

pub struct SocketServer {
    listener: TcpListener,
    data_sender: broadcast::Sender<SocketMessage>,
    clients: Arc<RwLock<HashMap<String, ClientInfo>>>,
    port: u16,
}

// ✅ FIX: Make ClientInfo public
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub address: String,
    pub connected_at: DateTime<Utc>,
    pub messages_sent: u64,
}

impl SocketServer {
    /// Create new socket server
    pub async fn new(port: u16) -> Result<Self, ModbusError> {
        let bind_address = format!("0.0.0.0:{}", port);
        let listener = TcpListener::bind(&bind_address).await
            .map_err(|e| ModbusError::CommunicationError(
                format!("Failed to bind socket server to {}: {}", bind_address, e)
            ))?;
        
        info!("🔌 Socket server listening on {}", bind_address);
        
        // Small buffer to avoid memory bloat - only keep last 50 messages
        let (data_sender, _) = broadcast::channel(50);
        
        Ok(Self {
            listener,
            data_sender,
            clients: Arc::new(RwLock::new(HashMap::new())),
            port,
        })
    }

    /// Start accepting client connections
    pub async fn start(&self) -> Result<(), ModbusError> {
        info!("🚀 Starting socket server on port {}", self.port);
        
        loop {
            match self.listener.accept().await {
                Ok((stream, addr)) => {
                    let client_id = format!("{}_{}", addr, Utc::now().timestamp());
                    let client_info = ClientInfo {
                        address: addr.to_string(),
                        connected_at: Utc::now(),
                        messages_sent: 0,
                    };
                    
                    // Add client to tracking
                    {
                        let mut clients = self.clients.write().await;
                        clients.insert(client_id.clone(), client_info);
                    }
                    
                    info!("🔗 New client connected: {} ({})", client_id, addr);
                    
                    // Spawn task to handle this client
                    let data_receiver = self.data_sender.subscribe();
                    let clients_ref = Arc::clone(&self.clients);
                    let client_id_clone = client_id.clone();
                    
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_client(stream, data_receiver, clients_ref, client_id_clone).await {
                            debug!("Client disconnected: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept client connection: {}", e);
                }
            }
        }
    }

    /// Handle individual client connection
    async fn handle_client(
        mut stream: TcpStream,
        mut data_receiver: broadcast::Receiver<SocketMessage>,
        clients: Arc<RwLock<HashMap<String, ClientInfo>>>,
        client_id: String,
    ) -> Result<(), ModbusError> {
        let peer_addr = stream.peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        
        // Send welcome message
        let welcome = serde_json::json!({
            "type": "welcome",
            "message": "Connected to Industrial Modbus Data Stream",
            "timestamp": Utc::now(),
            "client_id": client_id
        });
        
        if let Err(e) = Self::send_message(&mut stream, &welcome.to_string()).await {
            warn!("Failed to send welcome message to {}: {}", peer_addr, e);
            return Err(e);
        }

        // Listen for data updates and forward to client
        while let Ok(message) = data_receiver.recv().await {
            let json_message = match serde_json::to_string(&message) {
                Ok(json) => json,
                Err(e) => {
                    warn!("Failed to serialize message: {}", e);
                    continue;
                }
            };

            if let Err(e) = Self::send_message(&mut stream, &json_message).await {
                debug!("Client {} disconnected: {}", peer_addr, e);
                break;
            }

            // Update client stats
            {
                let mut clients_guard = clients.write().await;
                if let Some(client_info) = clients_guard.get_mut(&client_id) {
                    client_info.messages_sent += 1;
                }
            }
        }

        // Remove client from tracking
        {
            let mut clients_guard = clients.write().await;
            clients_guard.remove(&client_id);
        }
        
        info!("❌ Client disconnected: {}", peer_addr);
        Ok(())
    }

    /// Send message to client (efficient, no buffering)
    async fn send_message(stream: &mut TcpStream, message: &str) -> Result<(), ModbusError> {
        let data = format!("{}\n", message);
        stream.write_all(data.as_bytes()).await
            .map_err(|e| ModbusError::CommunicationError(format!("Socket write error: {}", e)))?;
        
        // No explicit flush for efficiency - let TCP handle it
        Ok(())
    }

    /// Broadcast device data to all connected clients (non-blocking)
    pub fn broadcast_device_data(
        &self,
        device_address: u8,
        device_name: &str,
        device_uuid: &str,
        device_data: &dyn DeviceData,
    ) {
        let message = SocketMessage {
            timestamp: Utc::now(),
            device_address,
            device_name: device_name.to_string(),
            device_uuid: device_uuid.to_string(),
            data: device_data.get_all_parameters().into_iter().collect(),
        };

        // Non-blocking send - if no clients, message is dropped (efficient)
        let _ = self.data_sender.send(message);
    }

    /// Get current client statistics
    pub async fn get_client_stats(&self) -> HashMap<String, ClientInfo> {
        self.clients.read().await.clone()
    }

    /// Get server port
    pub fn port(&self) -> u16 {
        self.port
    }
}