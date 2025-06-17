use log::{error, info};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::{interval, sleep, Duration};

use crate::config::Config;
use crate::devices::{Device, DeviceData, FlowmeterDevice};
use crate::modbus::ModbusClient;
use crate::output::{DataFormatter, DataSender, ConsoleFormatter, ConsoleSender};
use crate::utils::error::ModbusError;

pub struct DataService {
    config: Config,
    devices: Vec<Box<dyn Device>>,
    device_data: Arc<Mutex<HashMap<u8, Box<dyn DeviceData>>>>,
    modbus_client: Arc<ModbusClient>,
    formatter: Box<dyn DataFormatter>,
    senders: Vec<Box<dyn DataSender>>,
}

impl DataService {
    pub async fn new(config: Config) -> Result<Self, ModbusError> {
        info!("🚀 Initializing Data Service");
        info!("📡 Target devices: {:?}", config.device_addresses);
        
        let modbus_client = ModbusClient::new(&config.serial_port, config.baud_rate, &config.parity)?;
        let mut devices: Vec<Box<dyn Device>> = Vec::new();

        // Initialize flowmeter devices
        for &addr in &config.device_addresses {
            let device = FlowmeterDevice::new(addr, format!("Flowmeter_{}", addr));
            devices.push(Box::new(device));
            info!("📋 Registered device address: {}", addr);
        }

        // Default output configuration
        let formatter: Box<dyn DataFormatter> = Box::new(ConsoleFormatter);
        let mut senders: Vec<Box<dyn DataSender>> = Vec::new();
        senders.push(Box::new(ConsoleSender));

        info!("✅ Data Service initialized successfully");
        Ok(Self {
            config,
            devices,
            device_data: Arc::new(Mutex::new(HashMap::new())),
            modbus_client: Arc::new(modbus_client),
            formatter,
            senders,
        })
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
        if let Ok(device_data) = self.device_data.lock() {
            let mut devices_vec = Vec::new();
            
            for &addr in &self.config.device_addresses {
                if let Some(data) = device_data.get(&addr) {
                    devices_vec.push((addr, data.as_ref()));
                }
            }

            if !devices_vec.is_empty() {
                let header = self.formatter.format_header();
                let formatted = self.formatter.format_multiple_devices(&devices_vec);
                let output = format!("{}{}", header, formatted);
                self.broadcast_data(&output).await?;
            } else {
                let message = "⚠️  No device data available";
                self.broadcast_data(message).await?;
            }
        }
        Ok(())
    }

    pub async fn print_volatile_data(&self, parameter: &str) -> Result<(), ModbusError> {
        if let Ok(device_data) = self.device_data.lock() {
            let mut values = HashMap::new();
            
            for &addr in &self.config.device_addresses {
                if let Some(data) = device_data.get(&addr) {
                    if let Some(value) = data.get_parameter(parameter) {
                        values.insert(addr, value);
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

    pub async fn print_single_device_data(&self, device_addr: u8) -> Result<(), ModbusError> {
        if let Ok(device_data) = self.device_data.lock() {
            if let Some(data) = device_data.get(&device_addr) {
                let header = self.formatter.format_header();
                let formatted = self.formatter.format_single_device(device_addr, data.as_ref());
                let output = format!("{}{}", header, formatted);
                self.broadcast_data(&output).await?;
            } else {
                let message = format!("❌ No data found for device: {}", device_addr);
                self.broadcast_data(&message).await?;
            }
        }
        Ok(())
    }

    // Keep existing methods
    pub async fn run(&self, debug_output: bool) -> Result<(), ModbusError> {
        if debug_output {
            info!("🚀 Starting continuous monitoring with automatic output");
            info!("📤 Output destinations: {}", 
                  self.senders.iter()
                      .map(|s| format!("{}({})", s.sender_type(), s.destination()))
                      .collect::<Vec<_>>()
                      .join(", "));
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
                        
                        if let Ok(mut device_data) = self.device_data.lock() {
                            device_data.insert(addr, data);
                        }
                        
                        info!("✅ Successfully read data from device {} ({})", addr, device.name());
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
                    // Add separator for readability
                    println!("{}", "─".repeat(80));
                }
            }
        }
    }

    pub async fn read_all_devices_once(&self) -> Result<(), ModbusError> {
        info!("📖 Reading data from all devices once...");
        
        for device in &self.devices {
            match device.read_data(self.modbus_client.as_ref()).await {
                Ok(data) => {
                    let addr = device.address();
                    
                    if let Ok(mut device_data) = self.device_data.lock() {
                        device_data.insert(addr, data);
                    }
                    
                    info!("✅ Successfully read data from device {} ({})", addr, device.name());
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

    // Keep existing methods
    pub fn get_all_device_data(&self) -> Vec<(u8, String)> {
        if let Ok(device_data) = self.device_data.lock() {
            self.config.device_addresses
                .iter()
                .filter_map(|&addr| {
                    device_data.get(&addr).map(|data| {
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
                    device_data.get(&addr)
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
}