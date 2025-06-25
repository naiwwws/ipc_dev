use chrono::{DateTime, Utc};
use clap::ArgMatches;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // IPC Identification
    pub ipc_uuid: String,               // NEW: Unique identifier for this IPC
    pub ipc_name: String,               // NEW: Human-readable name for this IPC
    pub ipc_version: String,            // NEW: Software version
    
    // Connection settings
    pub serial_port: String,
    pub baud_rate: u32,
    pub timeout_ms: u64,
    pub parity: ParityConfig,
    
    // Monitoring settings
    pub update_interval_seconds: u64,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    
    // Device configuration
    pub devices: Vec<DeviceConfig>,
    
    // Data collection settings
    pub data_collection: DataCollectionConfig,
    
    // Output settings
    pub output: OutputConfig,
    
    // Metadata for payload merging
    pub site_info: SiteInfo,
    
    // Legacy compatibility
    pub device_addresses: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub uuid: String,                   // Device UUID
    pub address: u8,                    // Modbus address
    pub device_type: String,            // Device type
    pub name: String,                   // Device name
    pub location: String,               // Physical location
    pub enabled: bool,                  // Whether device is enabled
    pub polling_interval: Option<u64>,  // Custom polling interval
    pub parameters: Vec<String>,        // Parameters to read
    pub metadata: HashMap<String, String>, // Additional metadata
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataCollectionConfig {
    pub batch_size: usize,
    pub buffer_size: usize,
    pub auto_save_interval: u64,
    pub include_raw_data: bool,
    pub include_timestamps: bool,
    pub include_metadata: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub default_format: String,
    pub file_output: Option<FileOutputConfig>,
    pub http_output: Option<HttpOutputConfig>,
    pub mqtt_output: Option<MqttOutputConfig>,
    pub database_output: Option<DatabaseOutputConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOutputConfig {
    pub enabled: bool,
    pub path: String,
    pub rotate: bool,
    pub max_file_size_mb: u64,
    pub compression: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpOutputConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub headers: HashMap<String, String>,
    pub timeout_seconds: u64,
    pub retry_attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttOutputConfig {
    pub enabled: bool,
    pub broker: String,
    pub topic_prefix: String,
    pub qos: u8,
    pub retain: bool,
    pub client_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseOutputConfig {
    pub enabled: bool,
    pub sqlite_config: SqliteConfig,
    pub batch_size: usize,
    pub flush_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteConfig {
    pub database_path: String,
    pub max_connections: u32,
    pub connection_timeout_seconds: u64,
    pub enable_wal: bool,
    pub cache_size: i32,
    pub auto_vacuum: bool,
    // Add missing fields that sqlite_manager.rs expects
    pub batch_size: usize,
    pub busy_timeout_ms: u64,
    pub wal_mode: bool,
    pub sync_mode: String,
    pub cache_size_kb: i32,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self {
            database_path: "data/sensor_data.db".to_string(),
            max_connections: 10,
            connection_timeout_seconds: 30,
            enable_wal: true,
            cache_size: 1000,
            auto_vacuum: true,
            // Add defaults for missing fields
            batch_size: 100,
            busy_timeout_ms: 30000,
            wal_mode: true,
            sync_mode: "NORMAL".to_string(),
            cache_size_kb: 1000,
        }
    }
}
impl Default for FileOutputConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "./data/sensor_data.json".to_string(),
            rotate: true,
            max_file_size_mb: 100,
            compression: false,
        }
    }
}

impl Default for DatabaseOutputConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sqlite_config: SqliteConfig::default(),
            batch_size: 100,
            flush_interval_seconds: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteInfo {
    pub site_id: String,
    pub site_name: String,
    pub location: String,
    pub timezone: String,
    pub operator: String,
    pub department: String,
    pub contact_email: String,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParityConfig {
    None,
    Even,
    Odd,
}

// Keep legacy RegisterConfig for backward compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterConfig {
    pub address: u16,
    pub count: u16,
    pub register_type: String,
    pub description: String,
}

impl Default for Config {
    fn default() -> Self {
        let ipc_uuid = Uuid::new_v4().to_string();
        let mut default_devices = Vec::new();
        
        // Device 1: Inlet Flowmeter
        default_devices.push(DeviceConfig {
            uuid: Uuid::new_v4().to_string(),
            address: 2,
            device_type: "Flowmeter".to_string(),
            name: "Inlet Flowmeter".to_string(),
            location: "Building A - Line 1".to_string(),
            enabled: true,
            polling_interval: None,
            parameters: vec![
                "MassFlowRate".to_string(),
                "Temperature".to_string(),
                "DensityFlow".to_string(),
                "VolumeFlowRate".to_string(),
            ],
            metadata: {
                let mut map = HashMap::new();
                map.insert("calibration_date".to_string(), "2024-01-15".to_string());
                map.insert("serial_number".to_string(), "TT_SB001-2024".to_string());
                map.insert("manufacturer".to_string(), "Sealand".to_string());
                map.insert("model".to_string(), "Coriolosis".to_string());
                map
            },
        });
        
        // Device 2: Outlet Flowmeter
        default_devices.push(DeviceConfig {
            uuid: Uuid::new_v4().to_string(),
            address: 3,
            device_type: "Flowmeter".to_string(),
            name: "Outlet Flowmeter".to_string(),
            location: "Building A - Line 2".to_string(),
            enabled: true,
            polling_interval: None,
            parameters: vec![
                "MassFlowRate".to_string(),
                "Temperature".to_string(),
                "DensityFlow".to_string(),
            ],
            metadata: {
                let mut map = HashMap::new();
                map.insert("calibration_date".to_string(), "2024-02-10".to_string());
                map.insert("serial_number".to_string(), "TT_SB002-2024".to_string());
                map.insert("manufacturer".to_string(), "Sealand".to_string());
                map.insert("model".to_string(), "Coriolosis".to_string());
                map
            },
        });

        Self {
            // IPC Identification
            ipc_uuid,
            ipc_name: "Industrial Data Collector".to_string(),
            ipc_version: crate::VERSION.to_string(),
            
            // Connection settings
            serial_port: "/dev/ttyS0".to_string(),
            baud_rate: 9600,
            timeout_ms: 1000,
            parity: ParityConfig::None,
            
            // Monitoring settings
            update_interval_seconds: 10,
            max_retries: 3,
            retry_delay_ms: 500,
            
            // Device configuration
            devices: default_devices.clone(),
            
            // Data collection settings
            data_collection: DataCollectionConfig {
                batch_size: 10,
                buffer_size: 100,
                auto_save_interval: 300,
                include_raw_data: false,
                include_timestamps: true,
                include_metadata: true,
            },
            
            // Output settings
            output: OutputConfig {
                default_format: "json".to_string(),
                file_output: Some(FileOutputConfig {
                    enabled: false,
                    path: "./data/sensor_data.json".to_string(),
                    rotate: true,
                    max_file_size_mb: 100,
                    compression: false,
                }),
                http_output: None,
                mqtt_output: None,
                database_output: Some(DatabaseOutputConfig {
                    enabled: true,
                    sqlite_config: SqliteConfig::default(),
                    batch_size: 100,
                    flush_interval_seconds: 60,
                }),
            },
            
            // Site information
            site_info: SiteInfo {
                site_id: "SITE_001".to_string(),
                site_name: "Industrial Plant A".to_string(),
                location: "Factory District, Industrial Zone".to_string(),
                timezone: "UTC+07:00".to_string(),
                operator: "Plant Operations Team".to_string(),
                department: "Production".to_string(),
                contact_email: "operations@company.com".to_string(),
                metadata: HashMap::new(),
            },
            
            // Legacy compatibility
            device_addresses: default_devices.iter().map(|d| d.address).collect(),
        }
    }
}

impl Config {
    pub fn from_matches(matches: &ArgMatches) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = Self::default();
        
        // Override with command line arguments
        config.serial_port = matches.get_one::<String>("port").unwrap().clone();
        config.baud_rate = matches.get_one::<String>("baud").unwrap().parse()?;
        config.update_interval_seconds = matches.get_one::<String>("interval").unwrap().parse()?;
        
        // Parse devices if provided
        if let Some(devices_str) = matches.get_one::<String>("devices") {
            let addresses: Vec<u8> = devices_str
                .split(',')
                .map(|s| s.trim().parse::<u8>())
                .collect::<Result<Vec<_>, _>>()?;
            
            // Update legacy device_addresses
            config.device_addresses = addresses.clone();
            
            // Update or create device configs
            for addr in addresses.clone() {
                if !config.devices.iter().any(|d| d.address == addr) {
                    config.devices.push(DeviceConfig {
                        uuid: Uuid::new_v4().to_string(),
                        address: addr,
                        device_type: "flowmeter".to_string(),
                        name: format!("Device {}", addr),
                        location: "Unknown".to_string(),
                        enabled: true,
                        polling_interval: None,
                        parameters: vec!["MassFlowRate".to_string(), "Temperature".to_string()],
                        metadata: HashMap::new(),
                    });
                }
            }
            
            // Remove devices not in the list
            config.devices.retain(|d| addresses.clone().contains(&d.address));
        }
        
        Ok(config)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;
        
        // Generate IPC UUID if not present (for backward compatibility)
        if config.ipc_uuid.is_empty() {
            config.ipc_uuid = Uuid::new_v4().to_string();
        }
        
        // Set default IPC name if not present
        if config.ipc_name.is_empty() {
            config.ipc_name = "Industrial Data Collector".to_string();
        }
        
        // Set version if not present
        if config.ipc_version.is_empty() {
            config.ipc_version = crate::VERSION.to_string();
        }
        
        // Ensure device_addresses is synced with devices
        config.device_addresses = config.devices.iter().map(|d| d.address).collect();
        
        Ok(config)
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        // Create directory if it doesn't exist
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    // NEW: IPC identification methods
    pub fn get_ipc_uuid(&self) -> &str {
        &self.ipc_uuid
    }

    pub fn get_ipc_name(&self) -> &str {
        &self.ipc_name
    }

    pub fn get_ipc_version(&self) -> &str {
        &self.ipc_version
    }

    pub fn set_ipc_name(&mut self, name: String) {
        self.ipc_name = name;
    }

    pub fn regenerate_ipc_uuid(&mut self) {
        self.ipc_uuid = Uuid::new_v4().to_string();
    }

    // Get list of enabled devices
    pub fn get_enabled_devices(&self) -> Vec<&DeviceConfig> {
        self.devices.iter().filter(|d| d.enabled).collect()
    }

    // Get device by address
    pub fn get_device_by_address(&self, address: u8) -> Option<&DeviceConfig> {
        self.devices.iter().find(|d| d.address == address)
    }

    // Get device by UUID
    pub fn get_device_by_uuid(&self, uuid: &str) -> Option<&DeviceConfig> {
        self.devices.iter().find(|d| d.uuid == uuid)
    }

    // Get device by name
    pub fn get_device_by_name(&self, name: &str) -> Option<&DeviceConfig> {
        self.devices.iter().find(|d| d.name == name)
    }

    // Legacy compatibility - get device addresses
    pub fn device_addresses(&self) -> Vec<u8> {
        self.get_enabled_devices().iter().map(|d| d.address).collect()
    }

    // Sync device_addresses with devices (call after modifying devices)
    pub fn sync_device_addresses(&mut self) {
        self.device_addresses = self.devices.iter().map(|d| d.address).collect();
    }

    // Create a new device with UUID
    pub fn create_new_device(&self, address: u8, device_id: String, device_type: String, name: String, location: String) -> DeviceConfig {
        DeviceConfig {
            uuid: Uuid::new_v4().to_string(),
            address,
            device_type,
            name,
            location,
            enabled: true,
            polling_interval: None,
            parameters: vec!["MassFlowRate".to_string(), "Temperature".to_string()],
            metadata: HashMap::new(),
        }
    }
}