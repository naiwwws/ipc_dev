use log::{error, info, warn, debug};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, interval, Duration};
use tokio::sync::mpsc;
use serde_json::json;
use crate::services::socket_server::WebSocketRequest;

use crate::config::Config;
use crate::modbus::ModbusClient;
use crate::devices::{Device, DeviceData, FlowmeterDevice};
use crate::output::{DataFormatter, DataSender, ConsoleFormatter, ConsoleSender};
use crate::output::raw_sender::{RawDataSender, RawDataFormat};
use crate::services::DatabaseService;
use crate::services::socket_server::WebSocketServer;
use crate::utils::error::ModbusError;
use tokio::sync::Mutex as TokioMutex;
use crate::devices::gps::{GpsService, GpsData};

pub struct DataService {
    config: Config,
    devices: Vec<Box<dyn Device>>,
    device_data: Arc<Mutex<HashMap<String, Box<dyn DeviceData>>>>,
    device_address_to_uuid: HashMap<u8, String>,
    modbus_client: Arc<ModbusClient>,
    formatter: Box<dyn DataFormatter>,
    senders: Vec<Box<dyn DataSender>>,
    database_service: Option<DatabaseService>,
    websocket_server: Option<Arc<WebSocketServer>>,
    streaming_clients: Arc<TokioMutex<usize>>,
    polling_handle: Arc<TokioMutex<Option<tokio::task::JoinHandle<()>>>>,
    
    // NEW: Transaction tracking with volume monitoring
    active_transaction: Arc<TokioMutex<Option<ActiveTransaction>>>,
    
    // NEW: GPS service
    gps_service: Option<GpsService>,
}

// NEW: Enhanced structure to track active transaction with timestamps and inventory
#[derive(Debug, Clone)]
struct ActiveTransaction {
    transaction_id: String,
    target_volume: f32,
    vessel_name: String,
    
    // NEW: Use VolumeTotal from flowmeter as reference
    start_volume_total: f32,    // VolumeTotal when transaction started
    current_volume_total: f32,  // Current VolumeTotal from flowmeter
    
    // Timestamps
    ts_start: i64,
    ts_end: Option<i64>,
    
    // Inventory values
    inv_start: f32,
    inv_end: Option<f32>,
}

impl Clone for DataService {
    fn clone(&self) -> Self {
        // Create new devices vector by recreating them from config
        let mut cloned_devices: Vec<Box<dyn Device>> = Vec::new();
        
        // Only clone devices if we have the config available
        for device_config in &self.config.devices {
            if device_config.enabled {
                let device = FlowmeterDevice::new(
                    device_config.address,
                    device_config.name.clone(),
                );
                cloned_devices.push(Box::new(device));
            }
        }

        Self {
            config: self.config.clone(),
            devices: cloned_devices, // Use properly cloned devices
            device_data: self.device_data.clone(),
            device_address_to_uuid: self.device_address_to_uuid.clone(),
            modbus_client: self.modbus_client.clone(),
            formatter: Box::new(ConsoleFormatter),
            senders: Vec::new(),
            database_service: self.database_service.clone(),
            websocket_server: self.websocket_server.clone(),
            streaming_clients: self.streaming_clients.clone(),
            polling_handle: self.polling_handle.clone(),
            active_transaction: self.active_transaction.clone(), // NEW field
            gps_service: self.gps_service.clone(), // NEW field
        }
    }
}

impl DataService {
    pub async fn new(config: Config) -> Result<Self, ModbusError> {
        info!("🚀 Initializing Data Service");
        info!("🏭 IPC: {} [{}]", config.get_ipc_name(), config.get_ipc_uuid());
        
        // Create modbus client
        let modbus_client = Arc::new(ModbusClient::new(&config.serial_port, config.baud_rate, &config.parity)?);
        
        // Create devices from config
        let mut devices: Vec<Box<dyn Device>> = Vec::new();
        let mut device_address_to_uuid = HashMap::new();

        // Initialize devices
        for device_config in &config.devices {
            if device_config.enabled {
                let device = FlowmeterDevice::new(
                    device_config.address, 
                    device_config.name.clone(),
                );
                devices.push(Box::new(device));
                device_address_to_uuid.insert(device_config.address, device_config.uuid.clone());
                
                info!("📋 Registered device '{}' at address {} with UUID: {}", 
                      device_config.name, 
                      device_config.address,
                      device_config.uuid);
            }
        }

        // Initialize and start database service
        let database_service = if config.output.database_output.as_ref().map(|db| db.enabled).unwrap_or(false) {
            match DatabaseService::new(config.clone()).await {
                Ok(db_service) => {
                    info!("💾 Database service initialized successfully");
                    Some(db_service)
                }
                Err(e) => {
                    error!("❌ Failed to initialize database service: {}", e);
                    None
                }
            }
        } else {
            info!("📝 Database service disabled");
            None
        };

        // Create formatter
        let formatter: Box<dyn DataFormatter> = Box::new(ConsoleFormatter);

        // Initialize senders
        let mut senders: Vec<Box<dyn DataSender>> = Vec::new();
        senders.push(Box::new(ConsoleSender));

        // Create WebSocket server if enabled
        let websocket_server = if config.socket_server.enabled {
            info!("🔌 Initializing WebSocket server on port {}", config.socket_server.port);
            let mut ws_server = WebSocketServer::new(config.clone());
            
            // Create channel for WebSocket-DataService communication
            let (ws_tx, ws_rx) = mpsc::channel::<WebSocketRequest>(100);
            
            // Set the channel in WebSocket server
            ws_server.set_data_service_channel(ws_tx);
            
            let ws_server_arc = Arc::new(ws_server);
            
            // Store the receiver for later use
            // We'll need to start the request handler in run() method
            
            Some((ws_server_arc, ws_rx))
        } else {
            info!("📝 WebSocket server disabled in config");
            None
        };

        // NEW: Initialize GPS service if enabled
        let gps_service = if config.gps.enabled {
            info!("🧭 Initializing GPS service on port {}", config.gps.port);
            let gps_service = GpsService::new(
                config.gps.port.clone(),
                config.gps.baud_rate,
            );
            
            // Auto-start if configured
            if config.gps.auto_start {
                if let Err(e) = gps_service.start().await {
                    warn!("⚠️ Failed to auto-start GPS service: {}", e);
                } else {
                    info!("🧭 GPS service auto-started");
                }
            }
            
            Some(gps_service)
        } else {
            info!("📝 GPS service disabled in config");
            None
        };

        // Create the DataService instance with new fields
        let data_service = Self {
            config,
            devices,
            device_data: Arc::new(Mutex::new(HashMap::new())),
            device_address_to_uuid,
            modbus_client,
            formatter,
            senders,
            database_service,
            websocket_server: websocket_server.as_ref().map(|(server, _)| server.clone()),
            streaming_clients: Arc::new(TokioMutex::new(0)),
            polling_handle: Arc::new(TokioMutex::new(None)),
            active_transaction: Arc::new(TokioMutex::new(None)), // NEW field
            gps_service,
        };

        // Start WebSocket request handler if WebSocket is enabled
        if let Some((_, ws_rx)) = websocket_server {
            let data_service_clone = data_service.clone();
            tokio::spawn(async move {
                data_service_clone.start_websocket_request_handler(ws_rx).await;
            });
        }

        Ok(data_service)
    }

    // Handler methods for WebSocket requests (KEEP ONLY ONE SET)
    async fn handle_flowmeter_read_request(
        &self, 
        client_id: String, 
        device_address: Option<u8>,
        request_id: String
    ) -> Result<(), ModbusError> {
        // If a specific device is requested, read only that device
        if let Some(addr) = device_address {
            if let Some(device) = self.devices.iter().find(|d| d.address() == addr) {
                info!("📡 Reading device {} for WebSocket client {}", addr, client_id);
                match device.read_data(self.modbus_client.as_ref()).await {
                    Ok(data) => {
                        // Send data to WebSocket client
                        if let Some(ws_server) = &self.websocket_server {
                            let mut response = json!({
                                "endpoint": "/flowmeter/read",
                                "status": "success",
                                "data": data.get_all_parameters(),
                                "timestamp": chrono::Utc::now().timestamp(),
                                "id": request_id
                            });
                            
                            // Add transaction progress if active
                            // if let Some(progress) = self.get_transaction_progress().await {
                            //     response["transaction_progress"] = progress;
                            // }
                            
                            ws_server.send_to_client(&client_id, &response).await?;
                        }
                    }
                    Err(e) => {
                        error!("❌ Failed to read device {}: {}", addr, e);
                        
                        if let Some(ws_server) = &self.websocket_server {
                            let error_response = json!({
                                "endpoint": "/flowmeter/read",
                                "status": "error",
                                "error": format!("Failed to read device {}: {}", addr, e),
                                "id": request_id
                            });
                            ws_server.send_to_client(&client_id, &error_response).await?;
                        }
                    }
                }
            }
        } else {
            // Read all devices
            info!("📡 Reading all devices for WebSocket client {}", client_id);
            let mut results = Vec::new();
            let mut success_count = 0;
            
            for device in &self.devices {
                match device.read_data(self.modbus_client.as_ref()).await {
                    Ok(data) => {
                        results.push((device.address(), data));
                        success_count += 1;
                    }
                    Err(e) => {
                        error!("❌ Failed to read from device {}: {}", device.address(), e);
                    }
                }
            }
            
            // Send combined response to the client
            if let Some(ws_server) = &self.websocket_server {
                let devices_data: Vec<serde_json::Value> = results
                    .iter()
                    .map(|(addr, data)| {
                        json!({
                            "device_address": *addr,
                            "timestamp": data.as_ref().unix_ts(),
                            "parameters": data.as_ref().get_parameters_as_floats()
                        })
                    })
                    .collect();
                
                let mut response = json!({
                    "endpoint": "/flowmeter/read",
                    "status": "success",
                    "timestamp": chrono::Utc::now().timestamp(),
                    "total_devices": self.devices.len(),
                    "successful_reads": success_count,
                    "devices": devices_data,
                    "id": request_id
                });
                
                // Add transaction progress if active
                // if let Some(progress) = self.get_transaction_progress().await {
                //     response["transaction_progress"] = progress;
                // }
                
                ws_server.send_to_client(&client_id, &response).await?;
            }
            
            info!("✅ Read {} of {} devices successfully for client {}", 
                  success_count, self.devices.len(), client_id);
        }
        
        Ok(())
    }

    async fn handle_recent_readings_request(
        &self, 
        client_id: String, 
        limit: i64,
        request_id: String
    ) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            let readings = db_service.get_recent_flowmeter_readings(limit).await?;
            
            if let Some(ws_server) = &self.websocket_server {
                let response = json!({
                    "endpoint": "/flowmeter/recent",
                    "status": "success",
                    "timestamp": chrono::Utc::now().timestamp(),
                    "count": readings.len(),
                    "data": readings,
                    "id": request_id
                });
                
                ws_server.send_to_client(&client_id, &response).await?;
                info!("📊 Sent {} recent flowmeter readings to client {}", readings.len(), client_id);
            }
        } else {
            // Database not available
            if let Some(ws_server) = &self.websocket_server {
                let error_response = json!({
                    "endpoint": "/flowmeter/recent",
                    "status": "error",
                    "error": "Database service not available",
                    "id": request_id
                });
                ws_server.send_to_client(&client_id, &error_response).await?;
            }
            return Err(ModbusError::ServiceNotAvailable("Database service not available".to_string()));
        }
        
        Ok(())
    }

    async fn handle_stats_request(
        &self, 
        client_id: String,
        request_id: String
    ) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            let stats = db_service.get_flowmeter_stats().await?;
            
            if let Some(ws_server) = &self.websocket_server {
                let response = json!({
                    "endpoint": "/flowmeter/stats",
                    "status": "success",
                    "timestamp": chrono::Utc::now().timestamp(),
                    "data": {
                        "total_readings": stats.total_readings,
                        "avg_mass_flow_rate": stats.avg_mass_flow_rate,
                        "max_mass_flow_rate": stats.max_mass_flow_rate,
                        "min_mass_flow_rate": stats.min_mass_flow_rate,
                        "avg_temperature": stats.avg_temperature,
                        "latest_timestamp": stats.latest_timestamp,
                        "earliest_timestamp": stats.earliest_timestamp
                    },
                    "id": request_id
                });
                
                ws_server.send_to_client(&client_id, &response).await?;
                info!("📊 Sent flowmeter statistics to client {}", client_id);
            }
        } else {
            // Database not available
            if let Some(ws_server) = &self.websocket_server {
                let error_response = json!({
                    "endpoint": "/flowmeter/stats",
                    "status": "error",
                    "error": "Database service not available",
                    "id": request_id
                });
                ws_server.send_to_client(&client_id, &error_response).await?;
            }
            return Err(ModbusError::ServiceNotAvailable("Database service not available".to_string()));
        }
        
        Ok(())
    }

    //  Handler methods for WebSocket requests (NEW: ADD MISSING METHODS)
    async fn handle_start_streaming_request(
        &self,
        client_id: String,
        device_address: Option<u8>,
        request_id: String
    ) -> Result<(), ModbusError> {
        info!("🎬 Starting streaming for client {} (device: {:?})", client_id, device_address);
        
        // Increment streaming clients counter and start polling if needed
        {
            let mut count = self.streaming_clients.lock().await;
            *count += 1;
            
            // Start polling if this is the first streaming client
            if *count == 1 {
                // Create a clone of self for the polling task
                let self_arc = Arc::new(self.clone());
                
                // Start polling in a separate task
                let handle = tokio::spawn(async move {
                    Self::start_polling_loop(self_arc).await;
                });
                
                // Store the task handle
                *self.polling_handle.lock().await = Some(handle);
                info!("▶️ Started polling loop for first streaming client");
            } else {
                info!("📡 Added streaming client (total: {})", *count);
            }
        }
        
        // Send confirmation to client
        if let Some(ws_server) = &self.websocket_server {
            let response = json!({
                "endpoint": "/flowmeter/read",
                "status": "streaming_confirmed",
                "message": "Streaming started. Real-time data will be sent automatically.",
                "timestamp": chrono::Utc::now().timestamp(),
                "streaming": true,
                "device_filter": device_address,
                "id": request_id
            });
            
            ws_server.send_to_client(&client_id, &response).await?;
        }
        
        Ok(())
    }

    async fn handle_stop_streaming_request(
        &self,
        client_id: String,
        request_id: String
    ) -> Result<(), ModbusError> {
        info!("🛑 Processing stop streaming request for client {}", client_id);
        
        // Clear active transaction when streaming stops
        {
            let mut active_tx = self.active_transaction.lock().await;
            if let Some(tx) = active_tx.take() {
                info!("🔗 Cleared active transaction {} (streaming stopped)", tx.transaction_id);
                
                // Update transaction status to 'cancelled' if not completed
                if let Some(db_service) = &self.database_service {
                    if let Err(e) = db_service.update_transaction_status(&tx.transaction_id, "cancelled").await {
                        error!("❌ Failed to update transaction status: {}", e);
                    }
                }
            }
        }
        
        // Decrement streaming clients counter and stop polling if no clients left
        let should_stop_polling = {
            let mut count = self.streaming_clients.lock().await;
            
            if *count > 0 {
                *count -= 1;
                info!("📡 Decremented streaming client count to {}", *count);
                
                if *count == 0 {
                    info!("⏹️ No more streaming clients - will stop polling loop");
                    true
                } else {
                    info!("📡 Still have {} streaming clients - keeping polling active", *count);
                    false
                }
            } else {
                warn!("⚠️ Received stop request but streaming count was already 0");
                false
            }
        };
    
        // Stop polling if this was the last client
        if should_stop_polling {
            if let Some(handle) = self.polling_handle.lock().await.take() {
                handle.abort();
                info!("✅ Successfully stopped polling loop (no streaming clients)");
            } else {
                warn!("⚠️ No polling handle found to stop");
            }
        }
        
        // Send confirmation to client
        if let Some(ws_server) = &self.websocket_server {
            let response = json!({
                "endpoint": "/flowmeter/stop",
                "status": "streaming_stopped",
                "message": "Streaming stopped successfully.",
                "timestamp": chrono::Utc::now().timestamp(),
                "streaming": false,
                "id": request_id
            });
            
            ws_server.send_to_client(&client_id, &response).await?;
            info!("📤 Sent stop confirmation to client {}", client_id);
        }
        
        Ok(())
    }

    //  Helper method to get device config by address
    fn get_device_config_by_address(&self, address: u8) -> Option<&crate::config::DeviceConfig> {
        self.config.devices.iter().find(|d| d.address == address)
    }

    //  Database storage method
    async fn store_device_data_to_database(
        &self,
        device_address: u8,
        device_data: &dyn DeviceData,
    ) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            if let Some(uuid) = self.get_uuid_from_address(device_address) {
                if let Some(_device_config) = self.get_device_config_by_address(device_address) {
                    db_service.store_device_data(
                        uuid,           
                        device_address,
                        device_data,
                    ).await?;
                    
                    info!("💾 Stored data for device {} to database", device_address);
                } else {
                    warn!("⚠️  Device config not found for address: {}", device_address);
                }
            } else {
                warn!("⚠️  UUID not found for device address: {}", device_address);
            }
        }
        Ok(())
    }

    // Helper method
    fn get_uuid_from_address(&self, address: u8) -> Option<&String> {
        self.device_address_to_uuid.get(&address)
    }

    //  Main continuous monitoring method
    pub async fn run(&self, debug_output: bool) -> Result<(), ModbusError> {
        info!("🚀 Starting service in endpoint-driven mode");
        info!("⚙️  Debug output: {}", if debug_output { "enabled" } else { "disabled" });
        
        // Database status
        if let Some(_) = &self.database_service {
            info!("💾 Database storage: ENABLED");
        } else {
            info!("📝 Database storage: DISABLED");
        }

        // Start WebSocket server if enabled
        if let Some(ws_server) = &self.websocket_server {
            let server_clone = Arc::clone(ws_server);
            tokio::spawn(async move {
                if let Err(e) = server_clone.start().await {
                    error!("❌ WebSocket server failed: {}", e);
                }
            });
            
            info!("🔌 WebSocket server: ENABLED on port {}", ws_server.port());
            info!("📡 Flowmeter reading will start when clients connect to /flowmeter/read");
            
            // Give the server time to start
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        } else {
            info!("📝 WebSocket server: DISABLED");
            return Err(ModbusError::ConnectionError("WebSocket server required for endpoint-driven mode".to_string()));
        }

        // Initial reading to verify devices are working (one-time only)
        info!("🔍 Performing initial device check...");
        self.read_all_devices_once().await?;
        
        info!("✅ Service started successfully");
        info!("⏱️  Update interval: {} seconds", self.config.update_interval_seconds);
        info!("⏳ Service ready. Waiting for WebSocket clients to start streaming...");
        
        // Keep the service running but don't poll automatically
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            
            // Log status periodically
            let client_count = *self.streaming_clients.lock().await;
            
            if client_count > 0 {
                info!("📊 Status: Active streaming to {} clients", client_count);
            } else {
                debug!("💤 Status: No active streaming (endpoint-driven mode)");
            }
        }
    }

    // Socket-based communication method
    pub async fn run_socket(&self, debug_output: bool) -> Result<(), ModbusError> {
        // Use the same implementation as run()
        self.run(debug_output).await
    }

    // Add method to trigger flowmeter readings via WebSocket
    pub async fn trigger_flowmeter_readings_via_websocket(&self) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            if let Some(ws_server) = &self.websocket_server {
                let readings = db_service.get_recent_flowmeter_readings(20).await?;
                ws_server.send_flowmeter_readings(readings).await?;
                info!("📡 Sent recent flowmeter readings via WebSocket");
            }
        }
        Ok(())
    }

    // Add method to send flowmeter stats via WebSocket
    pub async fn send_flowmeter_stats_via_websocket(&self) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            if let Some(ws_server) = &self.websocket_server {
                let stats = db_service.get_flowmeter_stats().await?;
                ws_server.send_flowmeter_stats(stats).await?;
                info!("📊 Sent flowmeter statistics via WebSocket");
            }
        }
        Ok(())
    }

    //  Fixed read_all_devices_once method
    pub async fn read_all_devices_once(&self) -> Result<(), ModbusError> {
        info!("📖 Reading data from all devices once...");
        
        let mut all_data = Vec::new();
        
        for device in &self.devices {
            match device.read_data(self.modbus_client.as_ref()).await {
                Ok(data) => {
                    let device_address = device.address();
                    
                    //  Clone the UUID to avoid borrowing issues
                    let device_uuid = self.get_uuid_from_address(device_address)
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string());
                    
                    //  Store in database if database service is available
                    if let Err(e) = self.store_device_data_to_database(device_address, data.as_ref()).await {
                        error!("Failed to store device data: {}", e);
                    }
                    
                    // Store in memory for immediate access
                    {
                        let mut device_data = self.device_data.lock().unwrap();
                        device_data.insert(device_uuid, data.clone_box());
                    }
                    all_data.push(data);
                }
                Err(e) => {
                    error!("Failed to read from device {}: {}", device.address(), e);
                }
            }
        }
        
        // Send to output formatters
        if !all_data.is_empty() {
            let formatted_data = all_data.iter()
                .map(|data| {
                    let device = self.devices.iter()
                        .find(|d| self.device_address_to_uuid.get(&d.address()).is_some())
                        .unwrap_or(&self.devices[0]);
                    self.formatter.format_single_device(device.address(), data.as_ref())
                })
                .collect::<Vec<_>>()
                .join("\n");
            
            for sender in &self.senders {
                if let Err(e) = sender.send(&formatted_data).await {
                    error!("Failed to send data: {}", e);
                }
            }
        }
        
        Ok(())
    }

    // Keep existing methods with fixes
    pub fn get_all_device_data(&self) -> Vec<(u8, String)> {
        if let Ok(device_data) = self.device_data.lock() {
            self.devices
                .iter()
                .filter_map(|device| {
                    let addr = device.address();
                    self.get_uuid_from_address(addr)
                        .and_then(|uuid| device_data.get(uuid))
                        .map(|data| {
                            let params = data.get_all_parameters();
                            let formatted = params.iter()
                                .map(|(name, value)| format!("{}: {}", name, value))
                                .collect::<Vec<_>>()
                                .join(", ");
                            (addr, formatted)
                        })
                })
                .collect()
        } else {
            vec![]
        }
    }

    pub fn get_volatile_data(&self, parameter: &str) -> String {
        if let Ok(device_data) = self.device_data.lock() {
            let values: Vec<String> = self.devices
                .iter()
                .filter_map(|device| {
                    let addr = device.address();
                    self.get_uuid_from_address(addr)
                        .and_then(|uuid| device_data.get(uuid))
                        .and_then(|data| data.get_parameter(parameter))
                })
                .collect();
            values.join(" ")
        } else {
            String::new()
        }
    }

    pub async fn reset_accumulation(&self, device_addr: u8) -> Result<(), ModbusError> {
        if let Some(device) = self.devices.iter().find(|d| d.address() == device_addr) {
            device.reset_accumulation(self.modbus_client.as_ref()).await
        } else {
            Err(ModbusError::InvalidDevice(device_addr))
        }
    }

    pub async fn read_raw_device_data(&self, device_addr: u8, format: &str, output_file: Option<&String>) -> Result<(), ModbusError> {
        for device in &self.devices {
            if device.address() == device_addr {
                if let Some(flowmeter) = device.as_any().downcast_ref::<FlowmeterDevice>() {
                    let raw_payload = flowmeter.read_raw_payload(self.modbus_client.as_ref()).await?;
                    
                    // Print to console
                    println!("🔍 Raw Data for Device {}:", device_addr);
                    println!("{}", raw_payload.debug_info());
                    println!("📊 Payload Size: {} bytes", raw_payload.payload_size);
                    println!("🌐 Online Decoder: {}", raw_payload.get_decoder_url());
                    
                    // Save to file if requested
                    if let Some(file_path) = output_file {
                        let raw_format = match format {
                            "hex" => RawDataFormat::Hex,
                            "binary" => RawDataFormat::Binary,
                            "json" => RawDataFormat::Json,
                            _ => RawDataFormat::Debug,
                        };
                        
                        let sender = RawDataSender::new(file_path, raw_format, true);
                        sender.send_raw_payload(&raw_payload).await?;
                        
                        info!("💾 Raw data saved to: {}", file_path);
                    }
                    
                    return Ok(());
                }
            }
        }
        
        Err(ModbusError::DeviceNotFound(format!("Device {} not found or not a flowmeter", device_addr)))
    }

    pub async fn read_all_raw_device_data(&self, format: &str, output_file: Option<&String>) -> Result<(), ModbusError> {
        let mut total_size = 0;
        
        for device in &self.devices {
            if let Some(flowmeter) = device.as_any().downcast_ref::<FlowmeterDevice>() {
                let raw_payload = flowmeter.read_raw_payload(self.modbus_client.as_ref()).await?;
                total_size += raw_payload.payload_size;
                
                println!("🔍 Raw Data for Device {}:", device.address());
                println!("{}", raw_payload.debug_info());
                println!("────────────────────────────────────────");
                
                // Save to file if requested
                if let Some(file_path) = output_file {
                    let device_file = format!("{}_{}", file_path, device.address());
                    let raw_format = match format {
                        "hex" => RawDataFormat::Hex,
                        "binary" => RawDataFormat::Binary,
                        "json" => RawDataFormat::Json,
                        _ => RawDataFormat::Debug,
                    };
                    
                    let sender = RawDataSender::new(&device_file, raw_format, true);
                    sender.send_raw_payload(&raw_payload).await?;
                }
            }
        }
        
        println!("📊 Total Raw Data Size: {} bytes across {} devices", total_size, self.devices.len());
        Ok(())
    }

    pub async fn compare_raw_vs_processed(&self, device_addr: u8) -> Result<(), ModbusError> {
        for device in &self.devices {
            if device.address() == device_addr {
                if let Some(flowmeter) = device.as_any().downcast_ref::<FlowmeterDevice>() {
                    let (processed_data, raw_payload) = flowmeter.read_data_with_raw(self.modbus_client.as_ref()).await?;
                    
                    println!("🔄 Raw vs Processed Data Comparison for Device {}:", device_addr);
                    println!("\n📊 Raw Data:");
                    println!("{}", raw_payload.debug_info());
                    
                    println!("\n📈 Processed Data:");
                    for (param, value) in processed_data.get_all_parameters() {
                        println!("  {}: {}", param, value);
                    }
                    
                    println!("\n🔬 Engineering Units from Raw:");
                    let engineering_from_raw = raw_payload.to_engineering_units();
                    for (param, value) in engineering_from_raw.get_all_parameters() {
                        println!("  {}: {}", param, value);
                    }
                    
                    return Ok(());
                }
            }
        }
        
        Err(ModbusError::DeviceNotFound(format!("Device {} not found", device_addr)))
    }

    pub async fn print_all_device_data(&self) -> Result<(), ModbusError> {
        let device_data = self.device_data.lock().unwrap();
        
        println!("📊 Current Device Data:");
        println!("{}", "=".repeat(80));
        
        for (uuid, data) in device_data.iter() {
            if let Some((address, device_config)) = self.device_address_to_uuid.iter()
                .find(|(_, u)| *u == uuid)
                .and_then(|(addr, _)| self.config.get_device_by_uuid(uuid).map(|config| (*addr, config)))
            {
                println!("🔧 Device: {} (Address: {}, UUID: {})", device_config.name, address, uuid);
                println!("📍 Location: {}", device_config.location);
                
                println!("📊 Data:");
                for (param, value) in data.get_all_parameters() {
                    println!("  {}: {}", param, value);
                }
                println!("{}", "-".repeat(40));
            }
        }
        
        Ok(())
    }

    //  CLI interface methods
    pub fn set_formatter(&mut self, formatter: Box<dyn DataFormatter>) {
        self.formatter = formatter;
        info!("🎨 Output formatter changed to: {}", self.formatter.formatter_type());
    }

    pub fn add_sender(&mut self, sender: Box<dyn DataSender>) {
        info!("📡 Adding output sender: {}", sender.sender_type());
        self.senders.push(sender);
    }

    pub fn get_database_service(&self) -> Option<&DatabaseService> {
        self.database_service.as_ref()
    }

    pub fn get_database_service_mut(&mut self) -> Option<&mut DatabaseService> {
        self.database_service.as_mut()
    }

    pub async fn check_database_health(&self) -> Result<Option<bool>, ModbusError> {
        if let Some(db_service) = &self.database_service {
            match db_service.get_flowmeter_stats().await {
                Ok(_) => Ok(Some(true)),
                Err(_) => Ok(Some(false)),
            }
        } else {
            Ok(None)
        }
    }

    // MINIMAL: Updated query method
    pub async fn query_flowmeter_data(&self, device_address: u8, limit: i64) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            let readings = db_service.get_device_flowmeter_readings(device_address, None, Some(limit)).await?;
            
            println!("📋 Recent flowmeter readings for device {}:", device_address);
            println!("{:<15} {:<15} {:<15} {:<15} {:<8} {:<25}", 
                "Mass Flow", "Temperature", "Density", "Vol Flow", "Error", "Unix Timestamp");
            println!("{}", "-".repeat(110));
            
            for reading in readings {
                println!("{:<15.2} {:<15.2} {:<15.4} {:<15.3} {:<8} {:<25}", 
                    reading.mass_flow_rate,
                    reading.temperature,
                    reading.density_flow,
                    reading.volume_flow_rate,
                    reading.error_code,
                    reading.unix_timestamp
                );
            }
        } else {
            println!("❌ Database service not enabled");
        }
        Ok(())
    }

    // MINIMAL: Updated stats method
    pub async fn get_flowmeter_stats(&self) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            let stats = db_service.get_flowmeter_stats().await?;
            
            println!("📊 Flowmeter Statistics:");
            println!("Total Readings: {}", stats.total_readings);
            if let Some(avg_flow) = stats.avg_mass_flow_rate {
                println!("Average Mass Flow Rate: {:.2}", avg_flow);
            }
            if let Some(max_flow) = stats.max_mass_flow_rate {
                println!("Maximum Mass Flow Rate: {:.2}", max_flow);
            }
            if let Some(min_flow) = stats.min_mass_flow_rate {
                println!("Minimum Mass Flow Rate: {:.2}", min_flow);
            }
            if let Some(avg_temp) = stats.avg_temperature {
                println!("Average Temperature: {:.2}", avg_temp);
            }
            if let Some(latest) = stats.latest_timestamp {
                println!("Latest Reading: {}", latest);
            }
        } else {
            println!("❌ Database service not enabled");
        }
        Ok(())
    }

    /// Returns the WebSocket port if the WebSocket server is enabled.
    pub fn get_websocket_port(&self) -> Option<u16> {
        self.websocket_server.as_ref().map(|s| s.port())
    }

    /// Returns the number of connected WebSocket clients.
    pub async fn get_websocket_client_count(&self) -> Option<usize> {
        if let Some(ws_server) = &self.websocket_server {
            Some(ws_server.get_client_count().await)
        } else {
            None
        }
    }

    /// Returns stats for all connected WebSocket clients.
    pub async fn get_websocket_client_stats(&self) -> Option<HashMap<String, crate::services::socket_server::ClientInfo>> {
        if let Some(ws_server) = &self.websocket_server {
            Some(ws_server.get_client_stats().await)
        } else {
            None
        }
    }

    // Helper method to get device address from UUID (if still needed)
    fn get_address_from_uuid(&self, uuid: &str) -> Option<u8> {
        for (address, device_uuid) in &self.device_address_to_uuid {
            if device_uuid == uuid {
                return Some(*address);
            }
        }
        None
    }

    // ADD THIS METHOD: Implementation of the missing start_polling_loop
    async fn start_polling_loop(service: Arc<DataService>) {
        let interval_duration = tokio::time::Duration::from_secs(service.config.update_interval_seconds);
        let mut interval = tokio::time::interval(interval_duration);

        info!("🔄 Started flowmeter polling loop with VolumeTotal-based transaction tracking");

        loop {
            interval.tick().await;
            
            // Check if we should still be polling
            {
                let count = *service.streaming_clients.lock().await;
                if count == 0 {
                    info!("⏹️ No streaming clients, stopping polling loop");
                    break;
                }
            }
            
            // Read from all devices
            for device in &service.devices {
                let addr = device.address();
                
                match device.read_data(service.modbus_client.as_ref()).await {
                    Ok(data) => {
                        let params = data.get_parameters_as_floats();
                        let current_volume_total = params.get("VolumeTotal").copied().unwrap_or(0.0);
                        
                        info!("📊 Device {} VolumeTotal: {:.2} L", addr, current_volume_total);

                        // Update transaction with current VolumeTotal
                        let active_transaction_id = service.update_transaction_with_volume_total(current_volume_total).await;

                        // Store to database with transaction ID
                        if let Some(db_service) = &service.database_service {
                            if let Some(uuid) = service.device_address_to_uuid.get(&addr) {
                                if let Err(e) = db_service.store_device_data_with_transaction(
                                    uuid, 
                                    addr, 
                                    data.as_ref(),
                                    active_transaction_id.clone()
                                ).await {
                                    error!("❌ Failed to store device {} data: {}", addr, e);
                                } else {
                                    debug!("✅ Stored device {} data with transaction: {:?}", addr, active_transaction_id);
                                }
                            }
                        }

                        // Update device data cache
                        if let Ok(mut device_data_map) = service.device_data.lock() {
                            if let Some(uuid) = service.device_address_to_uuid.get(&addr) {
                                device_data_map.insert(uuid.clone(), data.clone_box());
                            }
                        }
                        
                        // Send to WebSocket clients with updated transaction progress
                        if let Some(ws_server) = &service.websocket_server {
                            let mut message = json!({
                                "endpoint": "/flowmeter/read",
                                "type": "flowmeter_data",
                                "device_address": addr,
                                "device_name": device.name(),
                                "timestamp": chrono::Utc::now().timestamp(),
                                "data": params
                            });
                            
                            // Add transaction info if active
                            if let Some(tx_id) = &active_transaction_id {
                                message["transaction_id"] = json!(tx_id);
                                
                                // Add progress info using VolumeTotal calculation
                                if let Some(active_tx) = service.active_transaction.lock().await.as_ref() {
                                    let current_time = chrono::Utc::now().timestamp();
                                    let current_inv = params.get("VolumeInventory").copied().unwrap_or(active_tx.inv_start);
                                    
                                    // Calculate dispensed volume and progress percentage
                                    let dispensed_volume = active_tx.current_volume_total - active_tx.start_volume_total;
                                    let progress_percentage = if active_tx.target_volume > 0.0 {
                                        (dispensed_volume / active_tx.target_volume) * 100.0
                                    } else {
                                        0.0
                                    };
                                    
                                    message["transaction_progress"] = json!({
                                        "current_volume": dispensed_volume,           // Dispensed volume (not total)
                                        "ts_start": active_tx.ts_start,
                                        "inv_start": active_tx.inv_start,
                                        "ts_end": active_tx.ts_end.unwrap_or(current_time),
                                        "inv_end": active_tx.inv_end.unwrap_or(current_inv),
                                        "progress_percentage": progress_percentage,
                                        "target_volume": active_tx.target_volume,
                                        "vessel_name": active_tx.vessel_name,
                                        // Debug info
                                        "debug": {
                                            "start_volume_total": active_tx.start_volume_total,
                                            "current_volume_total": active_tx.current_volume_total,
                                            "dispensed_volume": dispensed_volume,
                                            "volume_flow_rate": params.get("VolumeFlowRate").copied().unwrap_or(0.0)
                                        }
                                    });
                                }
                            }
                            
                            if let Err(e) = ws_server.broadcast_to_endpoint("/flowmeter/read", &message).await {
                                error!("❌ Failed to send data to WebSocket clients: {}", e);
                            } else {
                                debug!("📡 Sent flowmeter data from device {} to clients", addr);
                            }
                        }
                    }
                    Err(e) => {
                        error!("❌ Failed to read from device {} ({}): {}", addr, device.name(), e);
                    }
                }
            }
        }
        
        info!("🛑 Polling loop stopped");
    }

    // ADD: Method to handle WebSocket requests (this is missing!)
    pub async fn handle_websocket_request(&self, request: WebSocketRequest) -> Result<(), ModbusError> {
        match request {
            WebSocketRequest::StartStreaming { client_id, device_address, request_id } => {
                self.handle_start_streaming_request(client_id, device_address, request_id).await
            }
            WebSocketRequest::StopStreaming { client_id, request_id } => {
                self.handle_stop_streaming_request(client_id, request_id).await
            }
            WebSocketRequest::ReadFlowmeter { client_id, device_address, request_id } => {
                self.handle_flowmeter_read_request(client_id, device_address, request_id).await
            }
            WebSocketRequest::GetRecentReadings { client_id, limit, request_id } => {
                // Handle recent readings request
                if let Some(db_service) = &self.database_service {
                    if let Some(ws_server) = &self.websocket_server {
                        match db_service.get_recent_flowmeter_readings(limit).await {
                            Ok(readings) => {
                                let response = json!({
                                    "endpoint": "/flowmeter/recent",
                                    "status": "success",
                                    "data": readings,
                                    "count": readings.len(),
                                    "timestamp": chrono::Utc::now().timestamp(),
                                    "id": request_id
                                });
                                ws_server.send_to_client(&client_id, &response).await?;
                            }
                            Err(e) => {
                                let error_response = json!({
                                    "endpoint": "/flowmeter/recent",
                                    "status": "error",
                                    "error": format!("Database error: {}", e),
                                    "id": request_id
                                });
                                ws_server.send_to_client(&client_id, &error_response).await?;
                            }
                        }
                    }
                }
                Ok(())
            }
            WebSocketRequest::GetStats { client_id, request_id } => {
                // Handle stats request
                if let Some(db_service) = &self.database_service {
                    if let Some(ws_server) = &self.websocket_server {
                        match db_service.get_flowmeter_stats().await {
                            Ok(stats) => {
                                let response = json!({
                                    "endpoint": "/flowmeter/stats",
                                    "status": "success",
                                    "data": stats,
                                    "timestamp": chrono::Utc::now().timestamp(),
                                    "id": request_id
                                });
                                ws_server.send_to_client(&client_id, &response).await?;
                            }
                            Err(e) => {
                                let error_response = json!({
                                    "endpoint": "/flowmeter/stats",
                                    "status": "error",
                                    "error": format!("Database error: {}", e),
                                    "id": request_id
                                });
                                ws_server.send_to_client(&client_id, &error_response).await?;
                            }
                        }
                    }
                }
                Ok(())
            }
        }
    }

    // ADD: Method to start WebSocket request processing loop
    pub async fn start_websocket_request_handler(&self, mut rx: mpsc::Receiver<WebSocketRequest>) {
        info!("🔄 Starting WebSocket request handler loop");
        
        while let Some(request) = rx.recv().await {
            if let Err(e) = self.handle_websocket_request(request).await {
                error!("❌ Failed to handle WebSocket request: {}", e);
            }
        }
        
        info!("🛑 WebSocket request handler stopped");
    }

    // NEW: Method to start transaction with volume tracking
    pub async fn start_transaction_with_volume(&self, transaction_id: String, target_volume: f32, vessel_name: String) {
        // Capture start inventory AND start VolumeTotal from device data
        let (inv_start, start_volume_total) = if let Ok(device_data_map) = self.device_data.lock() {
            if let Some((_, data)) = device_data_map.iter().next() {
                let params = data.get_parameters_as_floats();
                let inv = params.get("VolumeInventory").copied().unwrap_or(0.0);
                let vol_total = params.get("VolumeTotal").copied().unwrap_or(0.0);
                (inv, vol_total)
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        let ts_start = chrono::Utc::now().timestamp();

        let mut active_tx = self.active_transaction.lock().await;
        *active_tx = Some(ActiveTransaction {
            transaction_id: transaction_id.clone(),
            target_volume,
            vessel_name: vessel_name.clone(),
            start_volume_total,           // NEW: Store start VolumeTotal
            current_volume_total: start_volume_total, // Initialize with start value
            ts_start,
            ts_end: None,
            inv_start,
            inv_end: None,
        });

        info!("🔗 Started transaction {} for vessel '{}' at {} with target volume: {:.2} L", 
              transaction_id, vessel_name, ts_start, target_volume);
        info!("📊 Start VolumeTotal: {:.2} L, Start inventory: {:.2} L", 
              start_volume_total, inv_start);
    }

    // Updated: Get current active transaction ID (only if volume not reached)
    pub async fn get_active_transaction_id(&self) -> Option<String> {
        if let Some(active_tx) = self.active_transaction.lock().await.as_ref() {
            if active_tx.current_volume_total < active_tx.target_volume {
                Some(active_tx.transaction_id.clone())
            } else {
                None
            }
        } else {
            None
        }
    }

    // Updated: Use VolumeTotal to calculate progress
    async fn update_transaction_with_volume_total(&self, current_volume_total: f32) -> Option<String> {
        let mut active_tx_guard = self.active_transaction.lock().await;
        
        if let Some(active_tx) = active_tx_guard.as_mut() {
            // Update current VolumeTotal
            active_tx.current_volume_total = current_volume_total;
            
            // Calculate dispensed volume and progress
            let dispensed_volume = current_volume_total - active_tx.start_volume_total;
            let progress_percentage = if active_tx.target_volume > 0.0 {
                (dispensed_volume / active_tx.target_volume) * 100.0
            } else {
                0.0
            };
            
            info!("📊 Transaction {} progress: VolumeTotal {:.2} → {:.2} L (dispensed: {:.2} L = {:.1}% of {:.2} L target)", 
                  active_tx.transaction_id, 
                  active_tx.start_volume_total, 
                  current_volume_total,
                  dispensed_volume,
                  progress_percentage, 
                  active_tx.target_volume);
            
            // Check if target volume reached
            if dispensed_volume >= active_tx.target_volume {
                let completed_tx_id = active_tx.transaction_id.clone();
                let vessel_name = active_tx.vessel_name.clone();
                
                // Set end timestamp and inventory
                let ts_end = chrono::Utc::now().timestamp();
                active_tx.ts_end = Some(ts_end);
                
                // Get current inventory as end inventory
                let inv_end = if let Ok(device_data_map) = self.device_data.lock() {
                    if let Some((_, data)) = device_data_map.iter().next() {
                        let params = data.get_parameters_as_floats();
                        params.get("VolumeInventory").copied().unwrap_or(active_tx.inv_start)
                    } else {
                        active_tx.inv_start
                    }
                } else {
                    active_tx.inv_start
                };
                active_tx.inv_end = Some(inv_end);
                
                let duration = ts_end - active_tx.ts_start;
                
                info!("🎯 Transaction {} for vessel '{}' COMPLETED! Target {:.2} L reached in {}s", 
                      completed_tx_id, vessel_name, active_tx.target_volume, duration);
                info!("📊 Final VolumeTotal: {:.2} L (dispensed: {:.2} L), Inventory change: {:.2} → {:.2} L", 
                      current_volume_total, dispensed_volume, active_tx.inv_start, inv_end);
                
                // Update transaction status in database
                if let Some(db_service) = &self.database_service {
                    if let Err(e) = db_service.update_transaction_status(&completed_tx_id, "completed").await {
                        error!("❌ Failed to update transaction status: {}", e);
                    }
                }
                
                // Clear active transaction
                *active_tx_guard = None;
                return Some(completed_tx_id);
            }
            
            // Return transaction ID if still active
            Some(active_tx.transaction_id.clone())
        } else {
            None
        }
    }
    
    // NEW: GPS control methods
    pub async fn get_current_gps_data(&self) -> Option<GpsData> {
        if let Some(gps_service) = &self.gps_service {
            // Try to get fresh GPS fix for transaction
            match gps_service.get_current_gps_fix().await {
                Ok(Some(data)) => Some(data),
                Ok(None) => {
                    // Fallback to last known data
                    let last_data = gps_service.get_current_data().await;
                    if last_data.has_valid_fix() {
                        Some(last_data)
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        } else {
            None
        }
    }
    
    // Remove the continuous GPS monitoring methods or make them no-op
    pub async fn start_gps_service(&self) -> Result<(), ModbusError> {
        if let Some(gps_service) = &self.gps_service {
            gps_service.start().await?;
            info!("🧭 GPS service ready for on-demand requests");
            Ok(())
        } else {
            Err(ModbusError::ServiceNotAvailable("GPS service not enabled in config".to_string()))
        }
    }
    
    pub async fn stop_gps_service(&self) -> Result<(), ModbusError> {
        if let Some(gps_service) = &self.gps_service {
            gps_service.stop().await?;
            info!("🧭 GPS service stopped");
            Ok(())
        } else {
            Err(ModbusError::ServiceNotAvailable("GPS service not enabled in config".to_string()))
        }
    }
    
    pub async fn get_gps_status(&self) -> Result<String, ModbusError> {
        if let Some(gps_service) = &self.gps_service {
            Ok(gps_service.get_status().await)
        } else {
            Ok("GPS not available".to_string())
        }
    }

    // NEW: Method to get current transaction progress (for API endpoints)
    // pub async fn get_transaction_progress(&self) -> Option<serde_json::Value> {
    //     if let Some(active_tx) = self.active_transaction.lock().await.as_ref() {
    //         // Get current inventory value
    //         let current_inv = if let Ok(device_data_map) = self.device_data.lock() {
    //             if let Some((_, data)) = device_data_map.iter().next() {
    //                 let params = data.get_parameters_as_floats();
    //                 params.get("VolumeInventory").copied().unwrap_or(active_tx.inv_start)
    //             } else {
    //                 active_tx.inv_start
    //             }
    //         } else {
    //             active_tx.inv_start
    //         };

    //         Some(json!({
    //             "current_volume": active_tx.current_volume,
    //             "ts_start": active_tx.ts_start,
    //             "inv_start": active_tx.inv_start,
    //             "ts_end": active_tx.ts_end.unwrap_or(chrono::Utc::now().timestamp()),
    //             "inv_end": active_tx.inv_end.unwrap_or(current_inv),
    //             "progress_percentage": (active_tx.current_volume / active_tx.target_volume) * 100.0,
    //             "target_volume": active_tx.target_volume,
    //             "vessel_name": active_tx.vessel_name
    //         }))
    //     } else {
    //         None
    //     }
    // }
}