use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;

use crate::devices::{DeviceData, FlowmeterData};

pub trait DataFormatter: Send + Sync {
    fn format_single_device(&self, addr: u8, data: &dyn DeviceData) -> String;
    fn format_multiple_devices(&self, devices_data: &[(u8, &dyn DeviceData)]) -> String;
    fn format_parameter_data(&self, parameter: &str, values: &HashMap<u8, String>) -> String;
    fn format_header(&self) -> String;
    
    //  Add the missing format method
    fn format(&self, data: &[&dyn DeviceData]) -> String {
        // Convert to the format expected by format_multiple_devices
        let devices_data: Vec<(u8, &dyn DeviceData)> = data.iter()
            .map(|device_data| (device_data.device_address(), *device_data))
            .collect();
        self.format_multiple_devices(&devices_data)
    }
}

pub struct ConsoleFormatter;

impl DataFormatter for ConsoleFormatter {
    fn format_single_device(&self, addr: u8, data: &dyn DeviceData) -> String {
        let params = data.get_all_parameters();
        let mut output = format!("🔹 Device {} Data:\n", addr);
        
        for (name, value) in params {
            output.push_str(&format!("{}: {}\n", name, value));
        }
        output
    }
    
    fn format_multiple_devices(&self, devices_data: &[(u8, &dyn DeviceData)]) -> String {
        let mut output = String::from("📊 All Device Data:\n");
        output.push_str(&"═".repeat(60));
        output.push('\n');
        
        for (addr, data) in devices_data {
            output.push_str(&self.format_single_device(*addr, *data));
            output.push_str(&"-".repeat(30));
            output.push('\n');
        }
        
        output
    }
    
    fn format_parameter_data(&self, parameter: &str, values: &HashMap<u8, String>) -> String {
        let mut output = format!("📈 Parameter: {}\n", parameter);
        
        for (addr, value) in values {
            output.push_str(&format!("  Device {}: {}\n", addr, value));
        }
        
        output
    }
    
    fn format_header(&self) -> String {
        format!("🚀 Industrial Modbus Data - {}\n", Utc::now().format("%Y-%m-%d %H:%M:%S"))
    }
}

pub struct JsonFormatter;

impl DataFormatter for JsonFormatter {
    fn format_single_device(&self, addr: u8, data: &dyn DeviceData) -> String {
        let json_data = serde_json::json!({
            "device_address": addr,
            "timestamp": Utc::now().to_rfc3339(),
            "data": data.to_json()
        });
        
        serde_json::to_string_pretty(&json_data).unwrap_or_default()
    }
    
    fn format_multiple_devices(&self, devices_data: &[(u8, &dyn DeviceData)]) -> String {
        let devices: Vec<Value> = devices_data.iter()
            .map(|(addr, data)| {
                serde_json::json!({
                    "device_address": addr,
                    "data": data.to_json()
                })
            })
            .collect();
        
        let result = serde_json::json!({
            "timestamp": Utc::now().timestamp(),
            "devices": devices
        });
        
        serde_json::to_string_pretty(&result).unwrap_or_default()
    }
    
    fn format_parameter_data(&self, parameter: &str, values: &HashMap<u8, String>) -> String {
        let result = serde_json::json!({
            "parameter": parameter,
            "timestamp": Utc::now().timestamp(),
            "values": values
        });
        
        serde_json::to_string_pretty(&result).unwrap_or_default()
    }
    
    fn format_header(&self) -> String {
        String::new() // JSON doesn't need headers
    }
}

pub struct CsvFormatter;

impl DataFormatter for CsvFormatter {
    fn format_single_device(&self, addr: u8, data: &dyn DeviceData) -> String {
        let params = data.get_all_parameters();
        let mut csv = String::new();
        let timestamp = Utc::now().to_rfc3339();
        
        for (name, value) in params {
            csv.push_str(&format!("{},{},{},{}\n", addr, name, value, timestamp));
        }
        
        csv
    }
    
    fn format_multiple_devices(&self, devices_data: &[(u8, &dyn DeviceData)]) -> String {
        let mut csv = String::new();
        let timestamp = Utc::now().to_rfc3339();
        
        for (addr, data) in devices_data {
            let params = data.get_all_parameters();
            for (name, value) in params {
                csv.push_str(&format!("{},{},{},{}\n", addr, name, value, timestamp));
            }
        }
        
        csv
    }
    
    fn format_parameter_data(&self, parameter: &str, values: &HashMap<u8, String>) -> String {
        let mut csv = String::new();
        let timestamp = Utc::now().to_rfc3339();
        
        for (addr, value) in values {
            csv.push_str(&format!("{},{},{},{}\n", addr, parameter, value, timestamp));
        }
        
        csv
    }
    
    fn format_header(&self) -> String {
        "Device,Parameter,Value,Timestamp\n".to_string()
    }
}

pub struct HexFormatter;

impl DataFormatter for HexFormatter {
    fn format_single_device(&self, addr: u8, data: &dyn DeviceData) -> String {
        // Try to get FlowmeterData and create hex representation
        if let Some(flowmeter_data) = data.as_any().downcast_ref::<FlowmeterData>() {
            format!(
                "🔍 Device {} Hex Data:\n\
                ErrorCode: 0x{:08X}\n\
                MassFlowRate: 0x{:08X} ({:.2} kg/h)\n\
                DensityFlow: 0x{:08X} ({:.4} kg/L)\n\
                Temperature: 0x{:08X} ({:.1}°C)\n\
                VolumeFlowRate: 0x{:08X} ({:.3} L/h)\n\
                MassTotal: 0x{:08X} ({:.2} kg)\n\
                VolumeTotal: 0x{:08X} ({:.3} L)\n\
                MassInventory: 0x{:08X} ({:.2} kg)\n\
                VolumeInventory: 0x{:08X} ({:.3} L)\n\
                Timestamp: {}\n",
                addr,
                flowmeter_data.error_code,
                flowmeter_data.mass_flow_rate.to_bits(), flowmeter_data.mass_flow_rate,
                flowmeter_data.density_flow.to_bits(), flowmeter_data.density_flow,
                flowmeter_data.temperature.to_bits(), flowmeter_data.temperature,
                flowmeter_data.volume_flow_rate.to_bits(), flowmeter_data.volume_flow_rate,
                flowmeter_data.mass_total.to_bits(), flowmeter_data.mass_total,
                flowmeter_data.volume_total.to_bits(), flowmeter_data.volume_total,
                flowmeter_data.mass_inventory.to_bits(), flowmeter_data.mass_inventory,
                flowmeter_data.volume_inventory.to_bits(), flowmeter_data.volume_inventory,
                flowmeter_data.timestamp.format("%Y-%m-%d %H:%M:%S%.3f")
            )
        } else {
            format!("🔍 Device {} - Hex format not available for this device type\n", addr)
        }
    }
    
    fn format_multiple_devices(&self, devices_data: &[(u8, &dyn DeviceData)]) -> String {
        let mut output = String::from("🔍 All Devices Hex Data:\n");
        output.push_str(&"═".repeat(80));
        output.push('\n');
        
        for (addr, data) in devices_data {
            output.push_str(&self.format_single_device(*addr, *data));
            output.push_str(&"─".repeat(80));
            output.push('\n');
        }
        
        output
    }
    
    fn format_parameter_data(&self, parameter: &str, values: &HashMap<u8, String>) -> String {
        let mut output = format!("🔍 Parameter '{}' Hex Values:\n", parameter);
        
        for (addr, value) in values {
            output.push_str(&format!("Device {}: {}\n", addr, value));
        }
        
        output
    }
    
    fn format_header(&self) -> String {
        format!("🔍 Hex Data Output - {}\n", Utc::now().format("%Y-%m-%d %H:%M:%S%.3f"))
    }
}