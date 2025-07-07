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
}

// KEEP: Clone implementation ABOVE the main impl block (this one is correct)
impl Clone for DataService {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            devices: Vec::new(), // Don't clone devices, create empty vec
            device_data: self.device_data.clone(),
            device_address_to_uuid: self.device_address_to_uuid.clone(),
            modbus_client: self.modbus_client.clone(),
            formatter: Box::new(ConsoleFormatter),
            senders: Vec::new(),
            database_service: self.database_service.clone(),
            websocket_server: self.websocket_server.clone(),
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

        // Create a channel for WebSocket server requests
        let (ws_tx, mut ws_rx) = mpsc::channel::<WebSocketRequest>(100);
        
        // Initialize WebSocket server if enabled and mode is websocket
        let websocket_server: Option<Arc<WebSocketServer>> = if config.socket_server.enabled 
            && config.socket_server.mode == "websocket" {
            let mut server = WebSocketServer::new(config.clone());
            // Set the sender channel
            server.set_data_service_channel(ws_tx.clone());
            info!("🔌 WebSocket server initialized on port {}", server.port());
            Some(Arc::new(server))
        } else if config.socket_server.enabled {
            info!("📡 Socket server mode: {}", config.socket_server.mode);
            None
        } else {
            None
        };

        // Create the DataService instance FIRST
        let data_service = Self {
            config,
            devices,
            device_data: Arc::new(Mutex::new(HashMap::new())),
            device_address_to_uuid,
            modbus_client,
            formatter,
            senders,
            database_service,
            websocket_server,
        };
        
        // FIX: Clone data_service for the spawned task instead of moving it into Arc
        let data_service_clone = data_service.clone();
        
        // Spawn a task to handle WebSocket requests
        tokio::spawn(async move {
            while let Some(request) = ws_rx.recv().await {
                match request {
                    WebSocketRequest::ReadFlowmeter { client_id, device_address, request_id } => {
                        if let Err(e) = data_service_clone.handle_flowmeter_read_request(client_id, device_address, request_id).await {
                            error!("Error handling flowmeter read request: {}", e);
                        }
                    },
                    WebSocketRequest::StartStreaming { client_id, device_address, request_id } => {
                        if let Err(e) = data_service_clone.handle_start_streaming_request(client_id, device_address, request_id).await {
                            error!("Error handling start streaming request: {}", e);
                        }
                    },
                    WebSocketRequest::StopStreaming { client_id, request_id } => {
                        if let Err(e) = data_service_clone.handle_stop_streaming_request(client_id, request_id).await {
                            error!("Error handling stop streaming request: {}", e);
                        }
                    },
                    WebSocketRequest::GetRecentReadings { client_id, limit, request_id } => {
                        if let Err(e) = data_service_clone.handle_recent_readings_request(client_id, limit, request_id).await {
                            error!("Error handling recent readings request: {}", e);
                        }
                    },
                    WebSocketRequest::GetStats { client_id, request_id } => {
                        if let Err(e) = data_service_clone.handle_stats_request(client_id, request_id).await {
                            error!("Error handling stats request: {}", e);
                        }
                    },
                }
            }
        });
        
        // FIX: Return the original data_service, not Arc
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
                            let response = json!({
                                "endpoint": "/flowmeter/read",
                                "status": "success",
                                "data": data.get_all_parameters(),
                                "timestamp": chrono::Utc::now().timestamp(),
                                "id": request_id
                            });
                            
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
                
                let response = json!({
                    "endpoint": "/flowmeter/read",
                    "status": "success",
                    "timestamp": chrono::Utc::now().timestamp(),
                    "total_devices": self.devices.len(),
                    "successful_reads": success_count,
                    "devices": devices_data,
                    "id": request_id
                });
                
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
        info!("🛑 Stopping streaming for client {}", client_id);
        
        if let Some(ws_server) = &self.websocket_server {
            let response = json!({
                "endpoint": "/flowmeter/stop",
                "status": "streaming_stopped_confirmed",
                "message": "Streaming stopped successfully.",
                "timestamp": chrono::Utc::now().timestamp(),
                "streaming": false,
                "id": request_id
            });
            
            ws_server.send_to_client(&client_id, &response).await?;
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
        info!("🚀 Starting continuous monitoring");
        
        if self.database_service.is_some() {
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
            // Give the server time to start
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        } else {
            info!("📝 WebSocket server: DISABLED");
        }

        let interval_duration = tokio::time::Duration::from_secs(self.config.update_interval_seconds);
        let mut interval = tokio::time::interval(interval_duration);
        let start_time = std::time::Instant::now();

        loop {
            interval.tick().await;
            
            for device in &self.devices {
                let addr = device.address();
                
                match device.read_data(self.modbus_client.as_ref()).await {
                    Ok(data) => {
                        if debug_output {
                            println!("{}", self.formatter.format_single_device(addr, data.as_ref()));
                        }
                        
                        // Store in memory
                        if let Some(uuid) = self.device_address_to_uuid.get(&addr) {
                            if let Ok(mut device_data) = self.device_data.lock() {
                                device_data.insert(uuid.clone(), data.as_ref().clone_box());
                            }
                        }
                        
                        // Store to database if enabled
                        if let Err(e) = self.store_device_data_to_database(addr, data.as_ref()).await {
                            error!("❌ Failed to store device {} data to database: {}", addr, e);
                        }
                        
                        // Send to WebSocket clients via /flowmeter/read endpoint
                        if let Some(ws_server) = &self.websocket_server {
                            if let Err(e) = ws_server.send_device_data(data.as_ref()).await {
                                error!("❌ Failed to send data to WebSocket clients: {}", e);
                            } else {
                                debug!("📡 Sent flowmeter data from device {} to /flowmeter/read endpoint", addr);
                            }
                        }
                        
                        info!("✅ Successfully read and processed data from device {} ({})", addr, device.name());
                    }
                    Err(e) => {
                        error!("❌ Failed to read from device {} ({}): {}", addr, device.name(), e);
                    }
                }
            }

            // Periodically send system status (every 10 cycles)
            if let Some(ws_server) = &self.websocket_server {
                let cycle_count = start_time.elapsed().as_secs() / self.config.update_interval_seconds;
                if cycle_count % 10 == 0 {
                    let uptime = start_time.elapsed().as_secs();
                    if let Err(e) = ws_server.send_system_status(self.devices.len(), uptime).await {
                        error!("❌ Failed to send system status: {}", e);
                    }
                }
            }
        }
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
                        device_data.insert(device_uuid, data.as_ref().clone_box());
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

    // Socket-based communication method
    pub async fn run_socket(&self, debug_output: bool) -> Result<(), ModbusError> {
        info!("🚀 Starting continuous monitoring");
        if self.database_service.is_some() {
            info!("💾 Database storage: ENABLED");
        } else {
            info!("📝 Database storage: DISABLED");
        }
        
        // FIX: Start WebSocket server if enabled (same as in run() method)
        if let Some(ws_server) = &self.websocket_server {
            let server_clone = Arc::clone(ws_server);
            tokio::spawn(async move {
                if let Err(e) = server_clone.start().await {
                    error!("❌ WebSocket server failed: {}", e);
                }
            });
            
            info!("🔌 WebSocket server: ENABLED on port {}", ws_server.port());
            // Give the server time to start
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        } else {
            info!("📝 WebSocket server: DISABLED");
        }

        let mut interval = interval(Duration::from_secs(self.config.update_interval_seconds));

        loop {
            interval.tick().await;
            info!("🔄 Requesting data from devices");
            
            let mut success_count = 0;
            for device in &self.devices {
                match device.read_data(self.modbus_client.as_ref()).await {
                    Ok(data) => {
                        let addr = device.address();
                        
                        // Store to in-memory cache
                        if let Some(uuid) = self.get_uuid_from_address(addr) {
                            if let Ok(mut device_data) = self.device_data.lock() {
                                device_data.insert(uuid.clone(), data.as_ref().clone_box());
                            }
                        }
                        
                        //  Store to database if enabled
                        if let Err(e) = self.store_device_data_to_database(addr, data.as_ref()).await {
                            error!("❌ Failed to store device {} data to database: {}", addr, e);
                        }
                        
                        // FIX: Use send_device_data instead of broadcast_device_data
                        if let Some(ws_server) = &self.websocket_server {
                            ws_server.send_device_data(data.as_ref()).await.ok();
                        }
                        
                        info!("✅ Successfully read and stored data from device {} ({})", addr, device.name());
                        success_count += 1;
                    }
                    Err(e) => {
                        error!("❌ Failed to read data from device {} ({}): {:?}", 
                               device.address(), device.name(), e);
                    }
                }
                
                sleep(Duration::from_millis(100)).await;
            }

            if debug_output && success_count > 0 {
                if let Err(e) = self.print_all_device_data().await {
                    error!("❌ Failed to print device data: {:?}", e);
                }
            }
        }
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
}