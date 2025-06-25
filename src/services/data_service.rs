use log::{error, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::{interval, sleep, Duration};

use crate::config::Config;
use crate::devices::{Device, DeviceData, FlowmeterDevice};
use crate::modbus::{ModbusClient, ModbusClientTrait};
use crate::output::{DataFormatter, DataSender, ConsoleFormatter, ConsoleSender};
use crate::utils::error::ModbusError;
use crate::devices::flowmeter::{FlowmeterRawPayload};
use crate::output::raw_sender::{RawDataSender, RawDataFormat};
use crate::services::DatabaseService; // Add this import

pub struct DataService {
    config: Config,
    devices: Vec<Box<dyn Device>>,
    device_data: Arc<Mutex<HashMap<String, Box<dyn DeviceData>>>>, // UUID-keyed
    device_address_to_uuid: HashMap<u8, String>, // NEW: Address -> UUID mapping for Modbus
    modbus_client: Arc<ModbusClient>,
    formatter: Box<dyn DataFormatter>,
    senders: Vec<Box<dyn DataSender>>,
    database_service: Option<DatabaseService>, // Add this field
}

impl DataService {
    pub async fn new(config: Config) -> Result<Self, ModbusError> {
        info!("🚀 Initializing Data Service");
        info!("🏭 IPC: {} [{}]", config.get_ipc_name(), config.get_ipc_uuid());
        info!("📦 Version: {}", config.get_ipc_version());
        info!("🏢 Site: {} ({})", config.site_info.site_name, config.site_info.site_id);
        info!("📡 Target devices: {} configured", config.devices.len());
        
        let modbus_client = ModbusClient::new(&config.serial_port, config.baud_rate, &config.parity)?;
        let mut devices: Vec<Box<dyn Device>> = Vec::new();
        let mut device_address_to_uuid = HashMap::new();

        // Initialize devices using the new DeviceConfig structure
        for device_config in &config.devices {
            if device_config.enabled {
                let device = FlowmeterDevice::new(
                    device_config.address, 
                    device_config.name.clone(),
                );
                devices.push(Box::new(device));
                
                // Create address -> UUID mapping for Modbus operations
                device_address_to_uuid.insert(device_config.address, device_config.uuid.clone());
                
                info!("📋 Registered device '{}' [{}] at address {} with UUID: {}", 
                      device_config.name, 
                      device_config.device_type,
                      device_config.address,
                      device_config.uuid);
            } else {
                info!("⏸️  Device '{}' at address {} is disabled", 
                      device_config.name, 
                      device_config.address);
            }
        }

        // ✅ Add database service initialization
        let database_service = if config.output.database_output
            .as_ref()
            .map(|db| db.enabled)
            .unwrap_or(false) 
        {
            info!("🗄️  Initializing database service...");
            match DatabaseService::new(config.clone()).await {
                Ok(mut db_service) => {
                    if let Err(e) = db_service.start().await {
                        error!("❌ Failed to start database service: {}", e);
                        None
                    } else {
                        info!("✅ Database service initialized successfully");
                        Some(db_service)
                    }
                }
                Err(e) => {
                    error!("❌ Failed to initialize database service: {}", e);
                    None
                }
            }
        } else {
            info!("📝 Database storage disabled in configuration");
            None
        };

        // Default output configuration
        let formatter: Box<dyn DataFormatter> = Box::new(ConsoleFormatter);
        let mut senders: Vec<Box<dyn DataSender>> = Vec::new();
        senders.push(Box::new(ConsoleSender));

        info!("✅ Data Service initialized successfully for IPC '{}'", config.get_ipc_name());
        Ok(Self {
            config,
            devices,
            device_data: Arc::new(Mutex::new(HashMap::new())),
            device_address_to_uuid,
            modbus_client: Arc::new(modbus_client),
            formatter,
            senders,
            database_service, // ✅ Add this field
        })
    }

    // Helper method to get UUID from device address
    fn get_uuid_from_address(&self, address: u8) -> Option<&String> {
        self.device_address_to_uuid.get(&address)
    }

    // Helper method to get device address from UUID
    fn get_address_from_uuid(&self, uuid: &str) -> Option<u8> {
        self.device_address_to_uuid.iter()
            .find_map(|(addr, u)| if u == uuid { Some(*addr) } else { None })
    }

    // NEW: Get IPC information
    pub fn get_ipc_info(&self) -> HashMap<String, String> {
        let mut info: HashMap<String, String> = HashMap::new();
        info.insert("ipc_uuid".to_string(), self.config.get_ipc_uuid().to_string());
        info.insert("ipc_name".to_string(), self.config.get_ipc_name().to_string());
        info.insert("ipc_version".to_string(), self.config.get_ipc_version().to_string());
        info.insert("site_id".to_string(), self.config.site_info.site_id.clone());
        info.insert("site_name".to_string(), self.config.site_info.site_name.clone());
        info.insert("location".to_string(), self.config.site_info.location.clone());
        info.insert("operator".to_string(), self.config.site_info.operator.clone());
        info.insert("device_count".to_string(), self.config.devices.len().to_string());
        info.insert("enabled_devices".to_string(), self.config.get_enabled_devices().len().to_string());
        info
    }

    // NEW: Print IPC information
    pub fn print_ipc_info(&self) -> Result<(), ModbusError> {
        info!("🏭 Industrial PC Information:");
        info!("═══════════════════════════════════════");
        info!("🆔 IPC UUID: {}", self.config.get_ipc_uuid());
        info!("🏷️  IPC Name: {}", self.config.get_ipc_name());
        info!("📦 Version: {}", self.config.get_ipc_version());
        info!("🏢 Site: {} ({})", self.config.site_info.site_name, self.config.site_info.site_id);
        info!("📍 Location: {}", self.config.site_info.location);
        info!("👤 Operator: {}", self.config.site_info.operator);
        info!("📧 Contact: {}", self.config.site_info.contact_email);
        info!("🏭 Department: {}", self.config.site_info.department);
        info!("🌐 Timezone: {}", self.config.site_info.timezone);
        info!("📡 Devices: {} total, {} enabled", 
              self.config.devices.len(), 
              self.config.get_enabled_devices().len());
        info!("🔌 Serial: {} @ {} baud", self.config.serial_port, self.config.baud_rate);
        info!("⏱️  Polling: {} second intervals", self.config.update_interval_seconds);
        Ok(())
    }

    // NEW: Print device information with address mapping
    pub fn print_device_info(&self) -> Result<(), ModbusError> {
        info!("📋 Configured Devices ({}):", self.config.devices.len());
        info!("═══════════════════════════════════════════════════════════");
        
        for device_config in &self.config.devices {
            let status = if device_config.enabled { "✅ ENABLED" } else { "❌ DISABLED" };
            info!("🏷️  Name: {}", device_config.name);
            info!("🆔 UUID: {}", device_config.uuid);
            info!("📡 Modbus Address: {} | Type: {} | Status: {}", 
                  device_config.address, 
                  device_config.device_type, 
                  status);
            info!("📍 Location: {}", device_config.location);
            
            if !device_config.parameters.is_empty() {
                info!("📊 Parameters: {}", device_config.parameters.join(", "));
            }
            
            if !device_config.metadata.is_empty() {
                info!("🏷️  Metadata:");
                for (key, value) in &device_config.metadata {
                    info!("   • {}: {}", key, value);
                }
            }
            
            if let Some(interval) = device_config.polling_interval {
                info!("⏱️  Custom Polling: {} seconds", interval);
            }
            
            info!("───────────────────────────────────────────────────────────");
        }

        Ok(())
    }

    // Output system management
    pub fn add_sender(&mut self, sender: Box<dyn DataSender>) {
        info!("📤 Added {} sender to {}", sender.sender_type(), sender.destination());
        self.senders.push(sender);
    }

    pub fn set_formatter(&mut self, formatter: Box<dyn DataFormatter>) {
        info!("🎨 Changed data formatter");
        self.formatter = formatter;
    }

    pub fn clear_senders(&mut self) {
        info!("🗑️  Cleared all data senders");
        self.senders.clear();
    }

    // Enhanced data broadcasting
    async fn broadcast_data(&self, data: &str) -> Result<(), ModbusError> {
        let mut success_count = 0;
        let mut error_count = 0;

        for sender in &self.senders {
            match sender.send(data).await {
                Ok(_) => {
                    info!("✅ Data sent via {} to {}", sender.sender_type(), sender.destination());
                    success_count += 1;
                }
                Err(e) => {
                    error!("❌ Failed to send data via {} to {}: {:?}", 
                           sender.sender_type(), sender.destination(), e);
                    error_count += 1;
                }
            }
        }

        info!("📊 Broadcast summary: {} successful, {} failed", success_count, error_count);
        Ok(())
    }

    // Enhanced data output methods
    pub async fn print_all_device_data(&self) -> Result<(), ModbusError> {
        let data = self.device_data.lock().unwrap();
        
        if data.is_empty() {
            info!("📊 No device data available for IPC '{}'", self.config.get_ipc_name());
            return Ok(());
        }

        info!("📊 Device Data from IPC '{}' [{}]:", self.config.get_ipc_name(), self.config.get_ipc_uuid());
        info!("═══════════════════════════════════════════════════════════");
        
        // Create devices_data vector for formatter
        let mut devices_data = Vec::new();
        
        for (uuid, device_data) in data.iter() {
            // Find device config by UUID for additional info
            if let Some(device_config) = self.config.get_device_by_uuid(uuid) {
                info!("🏷️  Device: {} [{}] (UUID: {})", 
                      device_config.name, 
                      device_config.device_type,
                      uuid);
                info!("📍 Location: {}", device_config.location);
                info!("📡 Modbus Address: {}", device_config.address);
                // Add to devices_data for formatter
                devices_data.push((device_config.address, device_data.as_ref()));
            } else {
                info!("🏷️  Device UUID: {}", uuid);
            }
        }
        
        // Use the formatter to format multiple devices
        if !devices_data.is_empty() {
            let header = self.formatter.format_header();
            let formatted_data = self.formatter.format_multiple_devices(&devices_data);
            let output = format!("{}{}", header, formatted_data);
            self.broadcast_data(&output).await?;
        }
        
        info!("───────────────────────────────────────────────────────────");
        Ok(())
    }

    pub async fn print_volatile_data(&self, parameter: &str) -> Result<(), ModbusError> {
        if let Ok(device_data) = self.device_data.lock() {
            let mut values = HashMap::new();
            
            // Use address -> UUID mapping to get data
            for &addr in &self.config.device_addresses {
                if let Some(uuid) = self.get_uuid_from_address(addr) {
                    if let Some(data) = device_data.get(uuid) {
                        if let Some(value) = data.get_parameter(parameter) {
                            values.insert(addr, value);
                        }
                    }
                }
            }

            if !values.is_empty() {
                let header = self.formatter.format_header();
                let formatted = self.formatter.format_parameter_data(parameter, &values);
                let output = format!("{}{}", header, formatted);
                self.broadcast_data(&output).await?;
            } else {
                let message = format!("❌ No data found for parameter: {}", parameter);
                self.broadcast_data(&message).await?;
            }
        }
        Ok(())
    }

    pub async fn print_single_device_data(&self, identifier: &str) -> Result<(), ModbusError> {
        if let Ok(device_data) = self.device_data.lock() {
            // Try to find device by UUID, address, or name
            let target_uuid = if let Some(device) = self.config.get_device_by_uuid(identifier) {
                &device.uuid
            } else if let Ok(address) = identifier.parse::<u8>() {
                if let Some(uuid) = self.get_uuid_from_address(address) {
                    uuid
                } else {
                    return Err(ModbusError::InvalidData(format!("Device with address {} not found", address)));
                }
            } else if let Some(device) = self.config.get_device_by_name(identifier) {
                &device.uuid
            } else {
                return Err(ModbusError::InvalidData(format!("Device '{}' not found", identifier)));
            };

            if let Some(data) = device_data.get(target_uuid) {
                if let Some(device_config) = self.config.get_device_by_uuid(target_uuid) {
                    info!("📊 Device Data for '{}':", device_config.name);
                    info!("🏷️  Type: {} | Address: {} | Location: {}", 
                          device_config.device_type, 
                          device_config.address, 
                          device_config.location);
                    info!("🆔 UUID: {}", device_config.uuid);
                    
                    let header = self.formatter.format_header();
                    let formatted = self.formatter.format_single_device(device_config.address, data.as_ref());
                    let output = format!("{}{}", header, formatted);
                    self.broadcast_data(&output).await?;
                } else {
                    return Err(ModbusError::InvalidData(format!("Device config for UUID {} not found", target_uuid)));
                }
            } else {
                return Err(ModbusError::InvalidData(format!("No data available for device '{}'", identifier)));
            }
        }
        Ok(())
    }

    // Keep existing methods with UUID fixes
    pub async fn run(&self, debug_output: bool) -> Result<(), ModbusError> {
        if debug_output {
            info!("🚀 Starting continuous monitoring with automatic output and database storage");
            info!("📤 Output destinations: {}", 
                  self.senders.iter()
                      .map(|s| format!("{}({})", s.sender_type(), s.destination()))
                      .collect::<Vec<_>>()
                      .join(", "));
            
            if self.database_service.is_some() {
                info!("💾 Database storage: ENABLED");
            } else {
                info!("📝 Database storage: DISABLED");
            }
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
                        
                        // ✅ Store to database if enabled
                        if let Err(e) = self.store_device_data_to_database(addr, data.as_ref()).await {
                            error!("❌ Failed to store device {} data to database: {}", addr, e);
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

            // Print data if debug mode is enabled
            if debug_output && success_count > 0 {
                info!("📊 Broadcasting data from {} devices", success_count);
                if let Err(e) = self.print_all_device_data().await {
                    error!("❌ Failed to broadcast device data: {:?}", e);
                } else {
                    println!("{}", "─".repeat(80));
                }
            }
        }
    }

    // Update single read method to store data
    pub async fn read_all_devices_once(&self) -> Result<(), ModbusError> {
        info!("📖 Reading data from all devices once...");
        
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
                    
                    // ✅ Store to database if enabled
                    if let Err(e) = self.store_device_data_to_database(addr, data.as_ref()).await {
                        error!("❌ Failed to store device {} data to database: {}", addr, e);
                    }
                    
                    info!("✅ Successfully read and stored data from device {} ({})", addr, device.name());
                }
                Err(e) => {
                    error!("❌ Failed to read data from device {} ({}): {:?}", 
                           device.address(), device.name(), e);
                }
            }
            
            sleep(Duration::from_millis(100)).await;
        }
        
        Ok(())
    }

    // Keep existing methods with UUID mapping fixes
    pub fn get_all_device_data(&self) -> Vec<(u8, String)> {
        if let Ok(device_data) = self.device_data.lock() {
            self.config.device_addresses
                .iter()
                .filter_map(|&addr| {
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
            let values: Vec<String> = self.config.device_addresses
                .iter()
                .filter_map(|&addr| {
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

    // Keep existing raw data methods unchanged - they already use device address correctly
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

    // Add method to store device data
    async fn store_device_data_to_database(
        &self,
        device_address: u8,
        device_data: &dyn DeviceData,
    ) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            if let Some(uuid) = self.get_uuid_from_address(device_address) {
                if let Some(device_config) = self.config.get_device_by_uuid(uuid) {
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
                    warn!("⚠️  Device config not found for UUID: {}", uuid);
                }
            } else {
                warn!("⚠️  UUID not found for device address: {}", device_address);
            }
        }
        Ok(())
    }

    // Use existing DatabaseService methods instead of duplicating
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

    pub async fn query_device_data(&self, device_address: u8, limit: i64) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            if let Some(uuid) = self.get_uuid_from_address(device_address) {
                let readings = db_service.get_device_readings(uuid, None, Some(limit)).await?;
                
                println!("📋 Recent readings for device {}:", device_address);
                println!("{:<20} {:<15} {:<10} {:<25}", "Parameter", "Value", "Unit", "Timestamp");
                println!("{}", "-".repeat(70));
                
                for reading in readings {
                    println!("{:<20} {:<15} {:<10} {:<25}", 
                        reading.parameter_name,
                        reading.parameter_value,
                        reading.parameter_unit.unwrap_or_else(|| "".to_string()),
                        reading.timestamp.format("%Y-%m-%d %H:%M:%S")
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

    pub async fn query_all_data(&self, limit: i64) -> Result<(), ModbusError> {
        if let Some(db_service) = &self.database_service {
            let readings = db_service.get_recent_readings(limit).await?;
            
            println!("📋 Recent readings (last {}):", limit);
            println!("{:<8} {:<20} {:<15} {:<10} {:<25}", "Device", "Parameter", "Value", "Unit", "Timestamp");
            println!("{}", "-".repeat(80));
            
            for reading in readings {
                println!("{:<8} {:<20} {:<15} {:<10} {:<25}", 
                    reading.device_address,
                    reading.parameter_name,
                    reading.parameter_value,
                    reading.parameter_unit.unwrap_or_else(|| "".to_string()),
                    reading.timestamp.format("%Y-%m-%d %H:%M:%S")
                );
            }
        } else {
            println!("❌ Database service not enabled");
        }
        Ok(())
    }
}