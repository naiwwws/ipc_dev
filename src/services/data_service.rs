use log::{error, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, interval, Duration};

use crate::config::Config;
use crate::modbus::ModbusClient;
use crate::devices::{Device, DeviceData, FlowmeterDevice};
use crate::output::{DataFormatter, DataSender, ConsoleFormatter, ConsoleSender};
use crate::output::raw_sender::{RawDataSender, RawDataFormat};
use crate::services::{DatabaseService, SocketServer};
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

    // ✅ ADD: Socket server
    socket_server: Option<Arc<SocketServer>>,
}

impl DataService {
    pub async fn new(config: Config) -> Result<Self, ModbusError> {
        info!("🚀 Initializing Data Service");
        info!("🏭 IPC: {} [{}]", config.get_ipc_name(), config.get_ipc_uuid());
        
        let modbus_client = ModbusClient::new(&config.serial_port, config.baud_rate, &config.parity)?;
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

        //  Initialize and start database service
        let database_service = if config.output.database_output.as_ref().map(|db| db.enabled).unwrap_or(false) {
            let mut db_service = DatabaseService::new(config.clone()).await?;
            db_service.start().await?;
            info!("💾 Database service initialized and started");
            Some(db_service)
        } else {
            info!("📝 Database service disabled");
            None
        };

        let formatter: Box<dyn DataFormatter> = Box::new(ConsoleFormatter);
        let mut senders: Vec<Box<dyn DataSender>> = Vec::new();
        senders.push(Box::new(ConsoleSender));

        // ✅ ADD: Initialize socket server if enabled
        let socket_server: Option<Arc<SocketServer>> = if config.socket_server.enabled {
            let server = SocketServer::new(config.clone()); // Pass entire config
            let server_arc = Arc::new(server);
            
            // Start server in background
            let server_clone = Arc::clone(&server_arc);
            tokio::spawn(async move {
                if let Err(e) = server_clone.start().await {
                    error!("Socket server error: {}", e);
                }
            });
            
            info!("🔌 Socket server enabled on port {}", config.socket_server.port);
            Some(server_arc)
        } else {
            info!("📝 Socket server disabled");
            None
        };

        Ok(DataService {
            config,
            devices,
            device_data: Arc::new(Mutex::new(HashMap::new())),
            device_address_to_uuid,
            modbus_client: Arc::new(modbus_client),
            formatter,
            senders,
            database_service,
            socket_server,
        })
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
                if let Some(device_config) = self.get_device_config_by_address(device_address) {
                    db_service.store_device_data(
                        uuid,
                        device_address,
                        &device_config.device_type,
                        &device_config.name,
                        &device_config.location,
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

    //  Fixed run method - use send_device_data instead of broadcast_device_data
    pub async fn run(&self, debug_output: bool) -> Result<(), ModbusError> {
        info!("🚀 Starting continuous monitoring");
        if self.database_service.is_some() {
            info!("💾 Database storage: ENABLED");
        } else {
            info!("📝 Database storage: DISABLED");
        }
        
        if self.socket_server.is_some() {
            info!("🔌 Socket streaming: ENABLED");
        } else {
            info!("📝 Socket streaming: DISABLED");
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
                        if let Some(socket_server) = &self.socket_server {
                            if let Err(e) = socket_server.send_device_data(data.as_ref()).await {
                                error!("❌ Failed to send data to socket clients: {}", e);
                            } else {
                                info!("📡 Sent data from device {} to socket clients", addr);
                            }
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
            let formatted_data = self.formatter.format(&all_data.iter().map(|d| d.as_ref()).collect::<Vec<_>>());
            
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
    }

    pub fn add_sender(&mut self, sender: Box<dyn DataSender>) {
        self.senders.push(sender);
    }

    pub fn get_database_service(&self) -> Option<&DatabaseService> {
        self.database_service.as_ref()
    }

    pub fn get_database_service_mut(&mut self) -> Option<&mut DatabaseService> {
        self.database_service.as_mut()
    }

    pub async fn get_database_stats(&self) -> Result<Option<crate::storage::DatabaseStats>, ModbusError> {
        if let Some(db_service) = &self.database_service {
            Ok(Some(db_service.get_stats().await?))
        } else {
            Ok(None)
        }
    }

    pub async fn check_database_health(&self) -> Result<Option<bool>, ModbusError> {
        if let Some(db_service) = &self.database_service {
            match db_service.get_stats().await {
                Ok(_) => Ok(Some(true)),
                Err(_) => Ok(Some(false)),
            }
        } else {
            Ok(None)
        }
    }

    pub async fn query_flowmeter_data(&self, device_address: u8, limit: i64) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            if let Some(uuid) = self.get_uuid_from_address(device_address) {
                let readings = db_service.get_device_flowmeter_readings(uuid, None, Some(limit)).await?;
                
                println!("📋 Recent flowmeter readings for device {}:", device_address);
                println!("{:<15} {:<15} {:<15} {:<15} {:<8} {:<10} {:<25}", 
                    "Mass Flow", "Temperature", "Density", "Vol Flow", "Error", "Quality", "Timestamp");
                println!("{}", "-".repeat(110));
                
                for reading in readings {
                    println!("{:<15.2} {:<15.2} {:<15.4} {:<15.3} {:<8} {:<10} {:<25}", 
                        reading.mass_flow_rate,     // f32 with 2 decimal places
                        reading.temperature,        // f32 with 2 decimal places
                        reading.density_flow,       // f32 with 4 decimal places
                        reading.volume_flow_rate,   // f32 with 3 decimal places
                        reading.error_code,
                        reading.quality_flag,
                        reading.unix_timestamp
                    );
                }
            } else {
                println!("❌ Device address {} not found", device_address);
            }
        } else {
            println!("❌ Database service not enabled");
        }
        Ok(())
    }

    pub async fn get_flowmeter_stats(&self) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            let stats = db_service.get_flowmeter_stats().await?;
            
            println!("📊 Flowmeter Statistics:");
            println!("Total Readings: {}", stats.total_readings);
            println!("Active Devices: {}", stats.active_devices);
            if let Some(avg_flow) = stats.avg_mass_flow_rate {
                println!("Average Mass Flow Rate: {:.2}", avg_flow);  // f32 with 2 decimal places
            }
            if let Some(max_flow) = stats.max_mass_flow_rate {
                println!("Maximum Mass Flow Rate: {:.2}", max_flow);  // f32 with 2 decimal places
            }
            if let Some(min_flow) = stats.min_mass_flow_rate {
                println!("Minimum Mass Flow Rate: {:.2}", min_flow);  // f32 with 2 decimal places
            }
            if let Some(avg_temp) = stats.avg_temperature {
                println!("Average Temperature: {:.2}", avg_temp);     // f32 with 2 decimal places
            }
            if let Some(latest) = stats.latest_reading {
                println!("Latest Reading: {}", latest);
            }
        } else {
            println!("❌ Database service not enabled");
        }
        Ok(())
    }

    // ✅ UPDATE: Add socket broadcasting to run method (around line 140)
    pub async fn run_socket(&self, debug_output: bool) -> Result<(), ModbusError> {
        info!("🚀 Starting continuous monitoring");
        if self.database_service.is_some() {
            info!("💾 Database storage: ENABLED");
        } else {
            info!("📝 Database storage: DISABLED");
        }
        
        if self.socket_server.is_some() {
            info!("🔌 Socket streaming: ENABLED");
        } else {
            info!("📝 Socket streaming: DISABLED");
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
                        if let Some(socket_server) = &self.socket_server {
                            if let Err(e) = socket_server.send_device_data(data.as_ref()).await {
                                error!("❌ Failed to send data to socket clients: {}", e);
                            } else {
                                info!("📡 Sent data from device {} to socket clients", addr);
                            }
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

    // ✅ ADD: Socket management methods
    pub async fn get_socket_client_stats(&self) -> Option<HashMap<String, crate::services::socket_server::ClientInfo>> {
        if let Some(socket_server) = &self.socket_server {
            Some(socket_server.get_client_stats().await)
        } else {
            None
        }
    }

    pub fn get_socket_port(&self) -> Option<u16> {
        self.socket_server.as_ref().map(|s| s.port())
    }
}