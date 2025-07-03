use log::{info, error, warn};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

use crate::config::Config;
use crate::devices::traits::DeviceData;
use crate::storage::models::FlowmeterReading;
use crate::utils::error::ModbusError;

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub address: String,
    pub connected_at: chrono::DateTime<chrono::Utc>,
    pub bytes_sent: u64,
    pub messages_sent: u64,
}

pub struct SocketServer {
    config: Config,
    clients: Arc<RwLock<HashMap<String, TcpStream>>>,
    client_stats: Arc<RwLock<HashMap<String, ClientInfo>>>,
    is_running: Arc<RwLock<bool>>,
    port: u16,
}

impl SocketServer {
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
        let bind_address = format!("{}:{}", "0.0.0.0", self.port);
        let listener = TcpListener::bind(&bind_address).await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to bind socket server: {}", e))
        })?;

        info!("🔌 Socket server listening on {}", bind_address);
        *self.is_running.write().await = true;

        while *self.is_running.read().await {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("🔗 New client connected: {}", addr);
                    let client_id = addr.to_string();
                    
                    // Add client info
                    let client_info = ClientInfo {
                        address: client_id.clone(),
                        connected_at: chrono::Utc::now(),
                        bytes_sent: 0,
                        messages_sent: 0,
                    };
                    
                    self.clients.write().await.insert(client_id.clone(), stream);
                    self.client_stats.write().await.insert(client_id, client_info);
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }

        Ok(())
    }

    // Update send_device_data to use Unix timestamp
    pub async fn send_device_data(&self, device_data: &dyn DeviceData) -> Result<(), ModbusError> {
        // Create JSON message with Unix timestamp
        let message = serde_json::json!({
            "type": "device_data",
            "timestamp": device_data.unix_timestamp(), // Unix timestamp as i64
            "device": {
                "type": device_data.device_type(),
                "address": device_data.device_address(),
                "name": device_data.device_name(),
                "location": device_data.device_location()
            },
            "data": device_data.get_parameters_as_floats()
        });

        self.broadcast_message(&message).await?;
        Ok(())
    }

    // Update send_flowmeter_data to use Unix timestamp
    pub async fn send_flowmeter_data(&self, reading: &FlowmeterReading) -> Result<(), ModbusError> {
        // Create JSON message with Unix timestamp
        let message = serde_json::json!({
            "type": "flowmeter_reading",
            "timestamp": reading.unix_timestamp, // Unix timestamp as i64
            "device": {
                "uuid": reading.device_uuid,
                "address": reading.device_address,
                "name": reading.device_name,
                "location": reading.device_location
            },
            "measurements": {
                "mass_flow_rate": reading.mass_flow_rate,
                "density_flow": reading.density_flow,
                "temperature": reading.temperature,
                "volume_flow_rate": reading.volume_flow_rate,
                "mass_total": reading.mass_total,
                "volume_total": reading.volume_total,
                "mass_inventory": reading.mass_inventory,
                "volume_inventory": reading.volume_inventory,
                "error_code": reading.error_code,
                "quality_flag": reading.quality_flag
            }
        });

        self.broadcast_message(&message).await?;
        Ok(())
    }

    async fn broadcast_message(&self, message: &Value) -> Result<(), ModbusError> {
        let message_str = format!("{}\n", serde_json::to_string(message).map_err(|e| {
            ModbusError::InvalidData(format!("Failed to serialize message: {}", e))
        })?);

        let mut clients = self.clients.write().await;
        let mut client_stats = self.client_stats.write().await;
        let mut clients_to_remove = Vec::new();

        for (client_id, stream) in clients.iter_mut() {
            match timeout(Duration::from_secs(5), stream.write_all(message_str.as_bytes())).await {
                Ok(Ok(())) => {
                    if let Err(e) = stream.flush().await {
                        warn!("Failed to flush data to client {}: {}", client_id, e);
                        clients_to_remove.push(client_id.clone());
                    } else {
                        // Update client stats
                        if let Some(stats) = client_stats.get_mut(client_id) {
                            stats.bytes_sent += message_str.len() as u64;
                            stats.messages_sent += 1;
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!("Failed to send data to client {}: {}", client_id, e);
                    clients_to_remove.push(client_id.clone());
                }
                Err(_) => {
                    warn!("Timeout sending data to client {}", client_id);
                    clients_to_remove.push(client_id.clone());
                }
            }
        }

        // Remove disconnected clients
        for client_id in clients_to_remove {
            clients.remove(&client_id);
            client_stats.remove(&client_id);
            info!("🔌 Client {} disconnected", client_id);
        }

        Ok(())
    }

    pub async fn stop(&self) -> Result<(), ModbusError> {
        info!("🛑 Stopping socket server...");
        *self.is_running.write().await = false;
        
        // Close all client connections
        let mut clients = self.clients.write().await;
        for (client_id, mut stream) in clients.drain() {
            if let Err(e) = stream.shutdown().await {
                warn!("Failed to shutdown client {}: {}", client_id, e);
            }
        }

        info!("✅ Socket server stopped");
        Ok(())
    }

    pub async fn get_client_count(&self) -> usize {
        self.clients.read().await.len()
    }

    pub async fn get_client_stats(&self) -> HashMap<String, ClientInfo> {
        self.client_stats.read().await.clone()
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}