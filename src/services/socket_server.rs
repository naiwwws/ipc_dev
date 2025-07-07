use log::{info, error, warn, debug};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, mpsc}; // ADD: mpsc import
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};
use futures_util::{SinkExt, StreamExt, stream::{SplitStream, SplitSink}}; // ADD: SplitSink import

use crate::config::Config;
use crate::devices::traits::DeviceData;
// REMOVE: use crate::devices::flowmeter::FlowmeterData; // This import might be causing issues
use crate::utils::error::ModbusError;
use crate::storage::models::{FlowmeterReading, FlowmeterStats};

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub address: String,
    pub connected_at: chrono::DateTime<chrono::Utc>,
    pub bytes_sent: u64,
    pub messages_sent: u64,
    pub subscribed_endpoints: Vec<String>,
}

// Define the WebSocketRequest type for communication with DataService
#[derive(Debug)]
pub enum WebSocketRequest {
    ReadFlowmeter { 
        client_id: String, 
        device_address: Option<u8>,
        request_id: String 
    },
    GetRecentReadings { 
        client_id: String, 
        limit: i64,
        request_id: String 
    },
    GetStats { 
        client_id: String,
        request_id: String  
    },
}

#[derive(Clone)]
pub struct WebSocketServer {
    config: Config,
    clients: Arc<RwLock<HashMap<String, SplitSink<WebSocketStream<TcpStream>, Message>>>>,
    client_stats: Arc<RwLock<HashMap<String, ClientInfo>>>,
    is_running: Arc<RwLock<bool>>,
    port: u16,
    data_service_tx: Option<Arc<mpsc::Sender<WebSocketRequest>>>, // FIX: Use tokio::sync::mpsc
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
            data_service_tx: None,
        }
    }

    // ADD: Method to set the communication channel with DataService
    pub fn set_data_service_channel(&mut self, sender: mpsc::Sender<WebSocketRequest>) {
        self.data_service_tx = Some(Arc::new(sender));
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
                                subscribed_endpoints: vec!["/flowmeter/read".to_string()],
                            };

                            // Create split stream for handling incoming messages
                            let (write, read) = ws_stream.split();
                            
                            // Store writer part in clients map
                            self.clients.write().await.insert(client_id.clone(), write);
                            self.client_stats.write().await.insert(client_id.clone(), client_info);

                            // Send welcome message with available endpoints
                            self.send_welcome_message(&client_id).await.ok();
                            
                            // Spawn a task to handle incoming messages from this client
                            let client_id_clone = client_id.clone();
                            let server = self.clone();
                            tokio::spawn(async move {
                                server.handle_client_messages(client_id_clone, read).await;
                            });
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

    // New method to handle incoming client messages
    async fn handle_client_messages(&self, client_id: String, mut read_half: SplitStream<WebSocketStream<TcpStream>>) {
        while let Some(msg_result) = read_half.next().await {
            match msg_result {
                Ok(msg) => {
                    if let Message::Text(text) = msg {
                        debug!("📥 Received message from client {}: {}", client_id, text);
                        
                        // Parse the message as JSON
                        if let Ok(json_msg) = serde_json::from_str::<serde_json::Value>(&text) {
                            // Check if this is an endpoint request
                            if let (Some(endpoint), Some(method)) = (
                                json_msg.get("endpoint").and_then(|v| v.as_str()),
                                json_msg.get("method").and_then(|v| v.as_str())
                            ) {
                                let request_id = json_msg.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                                
                                // Handle different endpoints
                                match endpoint {
                                    "/flowmeter/read" => {
                                        self.handle_flowmeter_read_endpoint(&client_id, method, &json_msg, request_id).await;
                                    },
                                    "/flowmeter/recent" => {
                                        self.handle_flowmeter_recent_endpoint(&client_id, method, &json_msg, request_id).await;
                                    },
                                    "/flowmeter/stats" => {
                                        self.handle_flowmeter_stats_endpoint(&client_id, method, &json_msg, request_id).await;
                                    },
                                    "/system/status" => {
                                        self.handle_system_status_endpoint(&client_id, method, request_id).await;
                                    },
                                    _ => {
                                        // Unknown endpoint
                                        let error_response = json!({
                                            "endpoint": endpoint,
                                            "status": "error",
                                            "error": "Unknown endpoint",
                                            "id": request_id
                                        });
                                        self.send_to_client(&client_id, &error_response).await.ok();
                                    }
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    warn!("❌ Error receiving message from client {}: {}", client_id, e);
                    break;
                }
            }
        }
        
        // Client disconnected
        info!("🔌 Client {} disconnected", client_id);
        let mut clients = self.clients.write().await;
        let mut stats = self.client_stats.write().await;
        clients.remove(&client_id);
        stats.remove(&client_id);
    }

    // Updated handler for /flowmeter/read endpoint to use the channel
    async fn handle_flowmeter_read_endpoint(&self, client_id: &str, method: &str, params: &Value, request_id: &str) {
        if method != "GET" {
            let error_response = json!({
                "endpoint": "/flowmeter/read",
                "status": "error",
                "error": "Method not supported. Use GET",
                "id": request_id
            });
            self.send_to_client(client_id, &error_response).await.ok();
            return;
        }

        // Extract device_address parameter if provided
        let device_address = params.get("params")
            .and_then(|p| p.get("device_address"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u8);

        // Send a preliminary response to the client
        let response = json!({
            "endpoint": "/flowmeter/read",
            "status": "pending",
            "message": format!(
                "Reading flowmeter data{}. Please wait...", 
                device_address.map_or("".to_string(), |addr| format!(" from device {}", addr))
            ),
            "timestamp": chrono::Utc::now().timestamp(),
            "id": request_id
        });
        
        self.send_to_client(client_id, &response).await.ok();
        
        // Send the request to DataService via the channel
        if let Some(tx) = &self.data_service_tx {
            let request = WebSocketRequest::ReadFlowmeter {
                client_id: client_id.to_string(),
                device_address,
                request_id: request_id.to_string(),
            };
            
            if let Err(e) = tx.send(request).await {
                error!("Failed to send flowmeter read request to DataService: {}", e);
                let error_response = json!({
                    "endpoint": "/flowmeter/read",
                    "status": "error",
                    "error": "Internal communication error",
                    "id": request_id
                });
                self.send_to_client(client_id, &error_response).await.ok();
            }
        } else {
            error!("No DataService channel available");
            let error_response = json!({
                "endpoint": "/flowmeter/read",
                "status": "error",
                "error": "Service not available",
                "id": request_id
            });
            self.send_to_client(client_id, &error_response).await.ok();
        }
    }

    // Similarly update other endpoint handlers...
    async fn handle_flowmeter_recent_endpoint(&self, client_id: &str, method: &str, params: &Value, request_id: &str) {
        if method != "GET" {
            let error_response = json!({
                "endpoint": "/flowmeter/recent",
                "status": "error",
                "error": "Method not supported. Use GET",
                "id": request_id
            });
            self.send_to_client(client_id, &error_response).await.ok();
            return;
        }

        // Get limit parameter if provided
        let limit = params.get("params")
            .and_then(|p| p.get("limit"))
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as i64;

        // Send the request to DataService via the channel
        if let Some(tx) = &self.data_service_tx {
            let request = WebSocketRequest::GetRecentReadings {
                client_id: client_id.to_string(),
                limit,
                request_id: request_id.to_string(),
            };
            
            if let Err(e) = tx.send(request).await {
                error!("Failed to send recent readings request to DataService: {}", e);
                let error_response = json!({
                    "endpoint": "/flowmeter/recent",
                    "status": "error",
                    "error": "Internal communication error",
                    "id": request_id
                });
                self.send_to_client(client_id, &error_response).await.ok();
            }
        } else {
            error!("No DataService channel available");
            let error_response = json!({
                "endpoint": "/flowmeter/recent",
                "status": "error",
                "error": "Service not available",
                "id": request_id
            });
            self.send_to_client(client_id, &error_response).await.ok();
        }
    }

    async fn handle_flowmeter_stats_endpoint(&self, client_id: &str, method: &str, _params: &Value, request_id: &str) {
        if method != "GET" {
            let error_response = json!({
                "endpoint": "/flowmeter/stats",
                "status": "error",
                "error": "Method not supported. Use GET",
                "id": request_id
            });
            self.send_to_client(client_id, &error_response).await.ok();
            return;
        }

        // Send the request to DataService via the channel
        if let Some(tx) = &self.data_service_tx {
            let request = WebSocketRequest::GetStats {
                client_id: client_id.to_string(),
                request_id: request_id.to_string(),
            };
            
            if let Err(e) = tx.send(request).await {
                error!("Failed to send stats request to DataService: {}", e);
                let error_response = json!({
                    "endpoint": "/flowmeter/stats",
                    "status": "error",
                    "error": "Internal communication error",
                    "id": request_id
                });
                self.send_to_client(client_id, &error_response).await.ok();
            }
        } else {
            error!("No DataService channel available");
            let error_response = json!({
                "endpoint": "/flowmeter/stats",
                "status": "error",
                "error": "Service not available",
                "id": request_id
            });
            self.send_to_client(client_id, &error_response).await.ok();
        }
    }

    async fn handle_system_status_endpoint(&self, client_id: &str, method: &str, request_id: &str) {
        if method != "GET" {
            let error_response = json!({
                "endpoint": "/system/status",
                "status": "error",
                "error": "Method not supported. Use GET",
                "id": request_id
            });
            self.send_to_client(client_id, &error_response).await.ok();
            return;
        }

        // Basic system status that we can provide directly from the WebSocketServer
        let response = json!({
            "endpoint": "/system/status",
            "status": "success",
            "data": {
                "websocket_server": {
                    "running": true,
                    "port": self.port,
                    "connected_clients": self.get_client_count().await,
                },
                "timestamp": chrono::Utc::now().timestamp(),
            },
            "id": request_id
        });
        
        self.send_to_client(client_id, &response).await.ok();
    }

    /// Send flowmeter data to all clients subscribed to /flowmeter/read endpoint
    pub async fn send_device_data(&self, device_data: &dyn DeviceData) -> Result<(), ModbusError> {
        // Check if this is flowmeter data
        if device_data.device_type() != "flowmeter" {
            return Ok(()); // Only handle flowmeter data
        }

        let message = serde_json::json!({
            "endpoint": "/flowmeter/read",
            "type": "flowmeter_data",
            "timestamp": device_data.unix_ts(),
            "device_address": device_data.device_address(),
            "data": device_data.get_parameters_as_floats()
        });

        self.broadcast_to_endpoint("/flowmeter/read", &message).await?;
        Ok(())
    }

    /// Send flowmeter readings from database
    pub async fn send_flowmeter_readings(&self, readings: Vec<FlowmeterReading>) -> Result<(), ModbusError> {
        let message = serde_json::json!({
            "endpoint": "/flowmeter/recent",
            "type": "flowmeter_readings",
            "timestamp": chrono::Utc::now().timestamp(),
            "count": readings.len(),
            "data": readings
        });

        self.broadcast_to_endpoint("/flowmeter/recent", &message).await?;
        Ok(())
    }

    /// Send flowmeter statistics
    pub async fn send_flowmeter_stats(&self, stats: FlowmeterStats) -> Result<(), ModbusError> {
        let message = serde_json::json!({
            "endpoint": "/flowmeter/stats",
            "type": "flowmeter_statistics",
            "timestamp": chrono::Utc::now().timestamp(),
            "data": {
                "total_readings": stats.total_readings,
                "avg_mass_flow_rate": stats.avg_mass_flow_rate,
                "max_mass_flow_rate": stats.max_mass_flow_rate,
                "min_mass_flow_rate": stats.min_mass_flow_rate,
                "avg_temperature": stats.avg_temperature,
                "latest_timestamp": stats.latest_timestamp,
                "earliest_timestamp": stats.earliest_timestamp
            }
        });

        self.broadcast_to_endpoint("/flowmeter/stats", &message).await?;
        Ok(())
    }

    /// Send system status
    pub async fn send_system_status(&self, device_count: usize, uptime_seconds: u64) -> Result<(), ModbusError> {
        let message = serde_json::json!({
            "endpoint": "/system/status",
            "type": "system_status",
            "timestamp": chrono::Utc::now().timestamp(),
            "data": {
                "service_running": true,
                "devices_configured": device_count,
                "websocket_enabled": true,
                "connected_clients": self.get_client_count().await,
                "uptime_seconds": uptime_seconds,
                "websocket_port": self.port
            }
        });

        self.broadcast_to_endpoint("/system/status", &message).await?;
        Ok(())
    }

    async fn send_welcome_message(&self, client_id: &str) -> Result<(), ModbusError> {
        let welcome_message = serde_json::json!({
            "endpoint": "/system/welcome",
            "type": "welcome",
            "timestamp": chrono::Utc::now().timestamp(),
            "data": {
                "message": "Connected to IPC WebSocket Server",
                "available_endpoints": [
                    "/flowmeter/read",
                    "/flowmeter/recent", 
                    "/flowmeter/stats",
                    "/system/status",
                    "/clients/list"
                ],
                "client_id": client_id
            }
        });

        self.send_to_client(client_id, &welcome_message).await
    }

    async fn broadcast_to_endpoint(&self, endpoint: &str, message: &Value) -> Result<(), ModbusError> {
        let message_text = serde_json::to_string(message).map_err(|e| {
            ModbusError::InvalidData(format!("Failed to serialize message: {}", e))
        })?;

        let mut clients = self.clients.write().await;
        let mut client_stats = self.client_stats.write().await;
        let mut clients_to_remove = Vec::new();

        for (client_id, ws_stream) in clients.iter_mut() {
            // Check if client is subscribed to this endpoint
            if let Some(stats) = client_stats.get(client_id) {
                if !stats.subscribed_endpoints.contains(&endpoint.to_string()) {
                    continue; // Skip clients not subscribed to this endpoint
                }
            }

            let ws_message = Message::Text(message_text.clone());
            match timeout(Duration::from_secs(5), ws_stream.send(ws_message)).await {
                Ok(Ok(())) => {
                    if let Some(stats) = client_stats.get_mut(client_id) {
                        stats.bytes_sent += message_text.len() as u64;
                        stats.messages_sent += 1;
                    }
                    debug!("📡 Sent {} data to client {}", endpoint, client_id);
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

        // Remove disconnected clients
        for client_id in clients_to_remove {
            clients.remove(&client_id);
            client_stats.remove(&client_id);
            info!("🔌 WebSocket client {} disconnected", client_id);
        }

        Ok(())
    }

    // FIX: Make send_to_client public and add an alias
    pub async fn send_response_to_client(&self, client_id: &str, message: &Value) -> Result<(), ModbusError> {
        self.send_to_client(client_id, message).await
    }

    pub async fn send_to_client(&self, client_id: &str, message: &Value) -> Result<(), ModbusError> {
        let message_text = serde_json::to_string(message).map_err(|e| {
            ModbusError::InvalidData(format!("Failed to serialize message: {}", e))
        })?;

        let mut clients = self.clients.write().await;
        let mut client_stats = self.client_stats.write().await;

        if let Some(ws_stream) = clients.get_mut(client_id) {
            let ws_message = Message::Text(message_text.clone());
            match timeout(Duration::from_secs(5), ws_stream.send(ws_message)).await {
                Ok(Ok(())) => {
                    if let Some(stats) = client_stats.get_mut(client_id) {
                        stats.bytes_sent += message_text.len() as u64;
                        stats.messages_sent += 1;
                    }
                }
                Ok(Err(e)) => {
                    warn!("Failed to send message to client {}: {}", client_id, e);
                    clients.remove(client_id);
                    client_stats.remove(client_id);
                }
                Err(_) => {
                    warn!("Timeout sending message to client {}", client_id);
                    clients.remove(client_id);
                    client_stats.remove(client_id);
                }
            }
        }

        Ok(())
    }

    pub async fn stop(&self) -> Result<(), ModbusError> {
        info!("🛑 Stopping WebSocket server...");
        *self.is_running.write().await = false;
        
        // Close all WebSocket connections
        let mut clients = self.clients.write().await;
        for (client_id, mut ws_stream) in clients.drain() {
            // FIX: Remove the None parameter from close()
            if let Err(e) = ws_stream.close().await {
                warn!("Failed to close WebSocket connection for client {}: {}", client_id, e);
            }
        }

        info!("✅ WebSocket server stopped");
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