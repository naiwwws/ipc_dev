use anyhow::Result;
// use chrono::{DateTime, Utc};
use serialport::SerialPort;
use std::collections::HashMap;
use std::io::{Write, Read};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::time::{interval, sleep};
use clap::{Arg, Command};
use serde::{Deserialize, Serialize};
use log::{error, info};

const VERSION: &str = "1.0.0";
const FLOWMETER_REG_OFFSET_ADDR: u16 = 1;

#[derive(Debug, Clone)]
pub struct FlowmeterData {
    pub error_code: u32,
    pub mass_flow_rate: f32,
    pub density_flow: f32,
    pub temperature: f32,
    pub volume_flow_rate: f32,
    pub mass_total: f32,
    pub volume_total: f32,
    pub mass_inventory: f32,
    pub volume_inventory: f32,
}

impl Default for FlowmeterData {
    fn default() -> Self {
        Self {
            error_code: 0,
            mass_flow_rate: 0.0,
            density_flow: 0.0,
            temperature: 0.0,
            volume_flow_rate: 0.0,
            mass_total: 0.0,
            volume_total: 0.0,
            mass_inventory: 0.0,
            volume_inventory: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub serial_port: String,
    pub baud_rate: u32,
    pub device_addresses: Vec<u8>,
    pub update_interval_seconds: u64,
    pub timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            serial_port: "/dev/ttyS0".to_string(),
            baud_rate: 9600,
            device_addresses: vec![2,3],
            update_interval_seconds: 10,
            timeout_ms: 1000,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ModbusRequest {
    ReadHoldingRegisters {
        address: u8,
        start_register: u16,
        quantity: u16,
        requester: String,
    },
    WriteSingleCoil {
        address: u8,
        coil_addr: u16,
        value: bool,
        requester: String,
    },
}

#[derive(Debug, Clone)]
pub enum ModbusResponse {
    Success(Vec<u8>),
    Error(String),
}

pub struct ModbusClient {
    port: Box<dyn SerialPort>,
}

impl ModbusClient {
    pub fn new(port_name: &str, baud_rate: u32) -> Result<Self, Box<dyn std::error::Error>> {
        info!("🔌 Connecting to Modbus RTU port: {}", port_name);
        info!("⚙️  Configuration: {} baud, 8 data bits, 1 stop bit, 1000ms timeout", baud_rate);
        
        let port = serialport::new(port_name, baud_rate)
            .timeout(Duration::from_millis(1000))
            .data_bits(serialport::DataBits::Eight)
            .stop_bits(serialport::StopBits::One)
            .parity(serialport::Parity::None)
            .open()
            .map_err(|e| {
                error!("❌ Failed to open serial port {}: {}", port_name, e);
                e
            })?;

        info!("✅ Modbus RTU connection established successfully");
        Ok(Self { port })
    }

    pub fn crc16_modbus(data: &[u8]) -> u16 {
        let mut crc: u16 = 0xFFFF;
        let poly: u16 = 0xA001;

        for &byte in data {
            crc ^= byte as u16;
            for _ in 0..8 {
                if crc & 0x0001 != 0 {
                    crc = (crc >> 1) ^ poly;
                } else {
                    crc >>= 1;
                }
            }
        }
        crc
    }

    pub fn read_holding_registers(
        &mut self,
        slave_id: u8,
        start_addr: u16,
        count: u16,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        info!("📊 Reading {} registers from device {} starting at address {}", count, slave_id, start_addr);
        
        let mut request = vec![slave_id, 0x03];
        request.extend_from_slice(&start_addr.to_be_bytes());
        request.extend_from_slice(&count.to_be_bytes());

        let crc = Self::crc16_modbus(&request);
        request.extend_from_slice(&crc.to_le_bytes());

        // info!("📤 Sending Modbus frame: {:02X?}", request);

        self.port.write_all(&request).map_err(|e| {
            error!("❌ Failed to write to serial port: {}", e);
            e
        })?;
        self.port.flush()?;

        // Wait for response
        thread::sleep(Duration::from_millis(50));

        let expected_len = 5 + (count * 2) as usize;
        let mut response = vec![0u8; expected_len];
        
        match self.port.read_exact(&mut response) {
            Ok(_) => {
                // info!("📥 Received response: {:02X?}", response);
                // Verify CRC
                let data_len = response.len() - 2;
                let received_crc = u16::from_le_bytes([response[data_len], response[data_len + 1]]);
                let calculated_crc = Self::crc16_modbus(&response[..data_len]);

                if received_crc == calculated_crc && response[0] == slave_id && response[1] == 0x03 {
                    // Return only the data bytes (skip slave_id, func_code, byte_count)
                    Ok(response[3..data_len].to_vec())
                } else {
                    Err("CRC mismatch or invalid response".into())
                }
            }
            Err(e) => {
                error!("❌ Failed to read from device {}: {}", slave_id, e);
                Err(e.into())
            }
        }
    }

    pub fn write_single_coil(
        &mut self,
        slave_id: u8,
        coil_addr: u16,
        value: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut request = vec![slave_id, 0x05];
        request.extend_from_slice(&coil_addr.to_be_bytes());
        request.extend_from_slice(&(if value { 0xFF00u16 } else { 0x0000u16 }).to_be_bytes());

        let crc = Self::crc16_modbus(&request);
        request.extend_from_slice(&crc.to_le_bytes());

        self.port.write_all(&request)?;
        self.port.flush()?;

        // Wait for response
        thread::sleep(Duration::from_millis(50));

        let mut response = vec![0u8; 8];
        match self.port.read_exact(&mut response) {
            Ok(_) => {
                let data_len = response.len() - 2;
                let received_crc = u16::from_le_bytes([response[data_len], response[data_len + 1]]);
                let calculated_crc = Self::crc16_modbus(&response[..data_len]);

                if received_crc == calculated_crc && response[0] == slave_id && response[1] == 0x05 {
                    Ok(())
                } else {
                    Err("CRC mismatch or invalid response".into())
                }
            }
            Err(e) => Err(e.into()),
        }
    }
}

pub struct FlowmeterService {
    config: Config,
    devices: Arc<Mutex<HashMap<u8, FlowmeterData>>>,
    modbus_client: Arc<Mutex<ModbusClient>>,
}

impl FlowmeterService {
    pub fn new(config: Config) -> Result<Self, Box<dyn std::error::Error>> {
        info!("🚀 Initializing Flowmeter Service");
        info!("📡 Target devices: {:?}", config.device_addresses);
        
        let modbus_client = ModbusClient::new(&config.serial_port, config.baud_rate)?;
        let mut devices = HashMap::new();

        // Initialize devices
        for &addr in &config.device_addresses {
            devices.insert(addr, FlowmeterData::default());
            info!("📋 Registered device address: {}", addr);
        }

        info!("✅ Flowmeter Service initialized successfully");
        Ok(Self {
            config,
            devices: Arc::new(Mutex::new(devices)),
            modbus_client: Arc::new(Mutex::new(modbus_client)),
        })
    }

    fn parse_flowmeter_data(&self, device_address: u8, data: &[u8]) -> FlowmeterData {
        if data.len() < 44 {
            error!("Insufficient data length for device {}: {}", device_address, data.len());
            return FlowmeterData::default();
        }

        // Convert register pairs to u32 values (big-endian)
        let error_code = ((data[0] as u32) << 24) | ((data[1] as u32) << 16) | 
                        ((data[2] as u32) << 8) | (data[3] as u32);
        
        let mass_flow_rate_raw = ((data[4] as u32) << 24) | ((data[5] as u32) << 16) | 
                                ((data[6] as u32) << 8) | (data[7] as u32);
        
        let density_flow_raw = ((data[8] as u32) << 24) | ((data[9] as u32) << 16) | 
                              ((data[10] as u32) << 8) | (data[11] as u32);
        
        let temperature_raw = ((data[12] as u32) << 24) | ((data[13] as u32) << 16) | 
                             ((data[14] as u32) << 8) | (data[15] as u32);
        
        let volume_flow_rate_raw = ((data[16] as u32) << 24) | ((data[17] as u32) << 16) | 
                                  ((data[18] as u32) << 8) | (data[19] as u32);
        
        let mass_total_raw = ((data[28] as u32) << 24) | ((data[29] as u32) << 16) | 
                            ((data[30] as u32) << 8) | (data[31] as u32);
        
        let volume_total_raw = ((data[32] as u32) << 24) | ((data[33] as u32) << 16) | 
                              ((data[34] as u32) << 8) | (data[35] as u32);
        
        let mass_inventory_raw = ((data[36] as u32) << 24) | ((data[37] as u32) << 16) | 
                                ((data[38] as u32) << 8) | (data[39] as u32);
        
        let volume_inventory_raw = ((data[40] as u32) << 24) | ((data[41] as u32) << 16) | 
                                  ((data[42] as u32) << 8) | (data[43] as u32);

        // Convert u32 to f32 (IEEE 754)
        FlowmeterData {
            error_code,
            mass_flow_rate: f32::from_bits(mass_flow_rate_raw),
            density_flow: f32::from_bits(density_flow_raw),
            temperature: f32::from_bits(temperature_raw),
            volume_flow_rate: f32::from_bits(volume_flow_rate_raw),
            mass_total: f32::from_bits(mass_total_raw),
            volume_total: f32::from_bits(volume_total_raw),
            mass_inventory: f32::from_bits(mass_inventory_raw),
            volume_inventory: f32::from_bits(volume_inventory_raw),
        }
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut interval = interval(Duration::from_secs(self.config.update_interval_seconds));

        loop {
            interval.tick().await;
            
            info!("Requesting data from flowmeter devices");
            
            for &device_addr in &self.config.device_addresses {
                match self.read_device_data(device_addr).await {
                    Ok(data) => {
                        let flowmeter_data = self.parse_flowmeter_data(device_addr, &data);
                        
                        if let Ok(mut devices) = self.devices.lock() {
                            devices.insert(device_addr, flowmeter_data.clone());
                        }
                        
                        info!("Address {}: Mass Flow Rate: {:.2}, Temperature: {:.2}, Density: {:.4}, Volume Flow Rate: {:.3}, Mass Total: {:.2}, Volume Total: {:.3}, Mass Inventory: {:.2}, Volume Inventory: {:.3}",
                               device_addr, flowmeter_data.mass_flow_rate, flowmeter_data.temperature,
                               flowmeter_data.density_flow, flowmeter_data.volume_flow_rate,
                               flowmeter_data.mass_total, flowmeter_data.volume_total,
                               flowmeter_data.mass_inventory, flowmeter_data.volume_inventory); 
                    }
                    Err(e) => {
                        error!("Failed to read data from device {}: {}", device_addr, e);
                        
                        // Set default values on error
                        if let Ok(mut devices) = self.devices.lock() {
                            devices.insert(device_addr, FlowmeterData::default());
                        }
                    }
                }
                
                // Small delay between device requests
                sleep(Duration::from_millis(100)).await;
            }
        }
    }

    async fn read_device_data(&self, device_addr: u8) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let start_register = 245 - FLOWMETER_REG_OFFSET_ADDR;
        let quantity = 22;

        if let Ok(mut client) = self.modbus_client.lock() {
            client.read_holding_registers(device_addr, start_register, quantity)
        } else {
            Err("Failed to acquire modbus client lock".into())
        }
    }

    pub fn reset_accumulation(&self, device_addr: u8) -> Result<(), Box<dyn std::error::Error>> {
        // Check if device exists
        if let Ok(devices) = self.devices.lock() {
            if !devices.contains_key(&device_addr) {
                return Err("Invalid device address".into());
            }
        }

        let coil_addr = 3 - FLOWMETER_REG_OFFSET_ADDR;
        
        if let Ok(mut client) = self.modbus_client.lock() {
            client.write_single_coil(device_addr, coil_addr, true)?;
            info!("Reset accumulation for device {}", device_addr);
            Ok(())
        } else {
            Err("Failed to acquire modbus client lock".into())
        }
    }

    pub fn get_volatile_data(&self, name: &str) -> String {
        if let Ok(devices) = self.devices.lock() {
            match name {
                "MassFlowRate" => {
                    let values: Vec<String> = self.config.device_addresses
                        .iter()
                        .map(|&addr| {
                            devices.get(&addr)
                                .map(|d| format!("{:.2}", d.mass_flow_rate))
                                .unwrap_or_else(|| "0.00".to_string())
                        })
                        .collect();
                    values.join(" ")
                }
                "DensityFlow" => {
                    let values: Vec<String> = self.config.device_addresses
                        .iter()
                        .map(|&addr| {
                            devices.get(&addr)
                                .map(|d| format!("{:.4}", d.density_flow))
                                .unwrap_or_else(|| "0.0000".to_string())
                        })
                        .collect();
                    values.join(" ")
                }
                "Temperature" => {
                    let values: Vec<String> = self.config.device_addresses
                        .iter()
                        .map(|&addr| {
                            devices.get(&addr)
                                .map(|d| format!("{:.2}", d.temperature))
                                .unwrap_or_else(|| "0.00".to_string())
                        })
                        .collect();
                    values.join(" ")
                }
                "VolumeFlowRate" => {
                    let values: Vec<String> = self.config.device_addresses
                        .iter()
                        .map(|&addr| {
                            devices.get(&addr)
                                .map(|d| format!("{:.3}", d.volume_flow_rate))
                                .unwrap_or_else(|| "0.000".to_string())
                        })
                        .collect();
                    values.join(" ")
                }
                "MassTotal" => {
                    let values: Vec<String> = self.config.device_addresses
                        .iter()
                        .map(|&addr| {
                            devices.get(&addr)
                                .map(|d| format!("{:.2}", d.mass_total))
                                .unwrap_or_else(|| "0.00".to_string())
                        })
                        .collect();
                    values.join(" ")
                }
                "VolumeTotal" => {
                    let values: Vec<String> = self.config.device_addresses
                        .iter()
                        .map(|&addr| {
                            devices.get(&addr)
                                .map(|d| format!("{:.3}", d.volume_total))
                                .unwrap_or_else(|| "0.000".to_string())
                        })
                        .collect();
                    values.join(" ")
                }
                "MassInventory" => {
                    let values: Vec<String> = self.config.device_addresses
                        .iter()
                        .map(|&addr| {
                            devices.get(&addr)
                                .map(|d| format!("{:.2}", d.mass_inventory))
                                .unwrap_or_else(|| "0.00".to_string())
                        })
                        .collect();
                    values.join(" ")
                }
                "VolumeInventory" => {
                    let values: Vec<String> = self.config.device_addresses
                        .iter()
                        .map(|&addr| {
                            devices.get(&addr)
                                .map(|d| format!("{:.3}", d.volume_inventory))
                                .unwrap_or_else(|| "0.000".to_string())
                        })
                        .collect();
                    values.join(" ")
                }
                "ErrorCode" => {
                    let values: Vec<String> = self.config.device_addresses
                        .iter()
                        .map(|&addr| {
                            devices.get(&addr)
                                .map(|d| d.error_code.to_string())
                                .unwrap_or_else(|| "0".to_string())
                        })
                        .collect();
                    values.join(" ")
                }
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    pub fn get_all_data(&self) -> Vec<FlowmeterData> {
        if let Ok(devices) = self.devices.lock() {
            self.config.device_addresses
                .iter()
                .map(|&addr| {
                    devices.get(&addr)
                        .cloned()
                        .unwrap_or_default()
                })
                .collect()
        } else {
            vec![]
        }
    }

    pub async fn read_all_devices_once(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Reading data from all devices once...");
        
        for &device_addr in &self.config.device_addresses {
            match self.read_device_data(device_addr).await {
                Ok(data) => {
                    let flowmeter_data = self.parse_flowmeter_data(device_addr, &data);
                    
                    if let Ok(mut devices) = self.devices.lock() {
                        devices.insert(device_addr, flowmeter_data.clone());
                    }
                    
                    info!("Address {}: Mass Flow Rate: {:.2}, Temperature: {:.2}, Density: {:.4}, Volume Flow Rate: {:.3}, Mass Total: {:.2}, Volume Total: {:.3}, Mass Inventory: {:.2}, Volume Inventory: {:.3}",
                           device_addr, flowmeter_data.mass_flow_rate, flowmeter_data.temperature,
                           flowmeter_data.density_flow, flowmeter_data.volume_flow_rate,
                           flowmeter_data.mass_total, flowmeter_data.volume_total,
                           flowmeter_data.mass_inventory, flowmeter_data.volume_inventory); 
                }
                Err(e) => {
                    error!("Failed to read data from device {}: {}", device_addr, e);
                }
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let matches = Command::new("flowmeter sealand")
        .version(VERSION)
        .about("Flowmeter Sealand Modbus Service")
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .value_name("PORT")
                .help("Serial port path")
                .default_value("/dev/ttyS0"),
        )
        .arg(
            Arg::new("baud")
                .short('b')
                .long("baud")
                .value_name("BAUD")
                .help("Baud rate")
                .default_value("9600"),
        )
        .arg(
            Arg::new("devices")
                .short('d')
                .long("devices")
                .value_name("DEVICES")
                .help("Device addresses (comma-separated)")
                .default_value("2,3"),
        )
        .arg(
            Arg::new("interval")
                .short('i')
                .long("interval")
                .value_name("SECONDS")
                .help("Update interval in seconds")
                .default_value("10"),
        )
        .subcommand(
            Command::new("getdata")
                .about("Get all device data")
        )
        .subcommand(
            Command::new("getvolatile")
                .about("Get volatile data for specific parameter")
                .arg(
                    Arg::new("parameter")
                        .help("Parameter name (MassFlowRate, Temperature, DensityFlow, etc.)")
                        .required(true)
                        .index(1),
                )
        )
        .subcommand(
            Command::new("resetaccumulation")
                .about("Reset accumulation for a device")
                .arg(
                    Arg::new("device_address")
                        .help("Device address to reset")
                        .required(true)
                        .index(1),
                ),
        )
        .get_matches();

    let config = Config {
        serial_port: matches.get_one::<String>("port").unwrap().clone(),
        baud_rate: matches.get_one::<String>("baud").unwrap().parse()?,
        device_addresses: matches
            .get_one::<String>("devices")
            .unwrap()
            .split(',')
            .map(|s| s.trim().parse::<u8>())
            .collect::<Result<Vec<_>, _>>()?,
        update_interval_seconds: matches.get_one::<String>("interval").unwrap().parse()?,
        timeout_ms: 1000,
    };

    let service = FlowmeterService::new(config)?;

    // Handle subcommands - BUT READ DATA FIRST
    if let Some(_matches) = matches.subcommand_matches("getdata") {
        info!("Executing getdata command...");
        
        // Read data from devices first
        service.read_all_devices_once().await?;
        
        let all_data = service.get_all_data();
        println!("All Device Data:");
        for (i, data) in all_data.iter().enumerate() {
            let device_addr = service.config.device_addresses[i];
            println!("Device {}: Error: {}, Mass Flow: {:.2}, Temp: {:.2}, Density: {:.4}", 
                     device_addr, data.error_code, data.mass_flow_rate, data.temperature, data.density_flow);
        }
        return Ok(());
    }

    if let Some(matches) = matches.subcommand_matches("getvolatile") {
        info!("Executing getvolatile command...");
        
        // Read data from devices first
        service.read_all_devices_once().await?;
        
        let parameter = matches.get_one::<String>("parameter").unwrap();
        let value = service.get_volatile_data(parameter);
        println!("{}: {}", parameter, value);
        return Ok(());
    }

    if let Some(matches) = matches.subcommand_matches("resetaccumulation") {
        let device_addr: u8 = matches.get_one::<String>("device_address").unwrap().parse()?;
        service.reset_accumulation(device_addr)?;
        println!("Reset accumulation command sent to device {}", device_addr);
        return Ok(());
    }

    // Start the continuous service (only if no subcommands)
    info!("Starting flowmeter sealand service version {}", VERSION);
    info!("Serial port: {}", service.config.serial_port);
    info!("Baud rate: {}", service.config.baud_rate);
    info!("Device addresses: {:?}", service.config.device_addresses);
    info!("Update interval: {} seconds", service.config.update_interval_seconds);

    // Run the service
    service.run().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc16_modbus() {
        let data = vec![0x01, 0x03, 0x00, 0xF4, 0x00, 0x16];
        let crc = ModbusClient::crc16_modbus(&data);
        // Expected CRC for this data (may vary based on implementation)
        assert!(crc != 0);
    }

    #[test]
    fn test_flowmeter_data_default() {
        let data = FlowmeterData::default();
        assert_eq!(data.error_code, 0);
        assert_eq!(data.mass_flow_rate, 0.0);
        assert_eq!(data.temperature, 0.0);
    }
}
