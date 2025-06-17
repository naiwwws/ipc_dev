use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::{error, info};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::any::Any;

use super::traits::{Device, DeviceData};
use crate::modbus::client::ModbusClientTrait;  // ← Fixed import path
use crate::utils::error::ModbusError;

const FLOWMETER_REG_OFFSET_ADDR: u16 = 1;

#[derive(Debug, Clone)]
pub struct FlowmeterDevice {
    pub address: u8,
    pub name: String,
    pub start_register: u16,
    pub register_count: u16,
    pub reset_coil: u16,
}

impl FlowmeterDevice {
    pub fn new(address: u8, name: String) -> Self {
        Self {
            address,
            name,
            start_register: 245 - FLOWMETER_REG_OFFSET_ADDR,
            register_count: 22,
            reset_coil: 3 - FLOWMETER_REG_OFFSET_ADDR,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowmeterData {
    pub device_address: u8,
    pub timestamp: DateTime<Utc>,
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
            device_address: 0,
            timestamp: Utc::now(),
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

#[async_trait]
impl Device for FlowmeterDevice {
    fn device_type(&self) -> &str {
        "BL410_Flowmeter"
    }
    
    fn address(&self) -> u8 {
        self.address
    }
    
    fn name(&self) -> &str {
        &self.name
    }
    
    async fn read_data(&self, client: &dyn ModbusClientTrait) -> Result<Box<dyn DeviceData>, ModbusError> {  // ← Changed signature
        let raw_data = client.read_holding_registers(self.address, self.start_register, self.register_count).await?;
        self.parse_raw_data(&raw_data)
    }
    
    async fn reset_accumulation(&self, client: &dyn ModbusClientTrait) -> Result<(), ModbusError> {  // ← Changed signature
        client.write_single_coil(self.address, self.reset_coil, true).await?;
        info!("Reset accumulation for device {} ({})", self.address, self.name);
        Ok(())
    }
    
    fn parse_raw_data(&self, data: &[u8]) -> Result<Box<dyn DeviceData>, ModbusError> {
        if data.len() < 44 {
            error!("Insufficient data length for device {}: {}", self.address, data.len());
            return Err(ModbusError::InvalidData("Insufficient data length".to_string()));
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

        let flowmeter_data = FlowmeterData {
            device_address: self.address,
            timestamp: Utc::now(),
            error_code,
            mass_flow_rate: f32::from_bits(mass_flow_rate_raw),
            density_flow: f32::from_bits(density_flow_raw),
            temperature: f32::from_bits(temperature_raw),
            volume_flow_rate: f32::from_bits(volume_flow_rate_raw),
            mass_total: f32::from_bits(mass_total_raw),
            volume_total: f32::from_bits(volume_total_raw),
            mass_inventory: f32::from_bits(mass_inventory_raw),
            volume_inventory: f32::from_bits(volume_inventory_raw),
        };
        
        Ok(Box::new(flowmeter_data))
    }
}

impl DeviceData for FlowmeterData {
    fn as_any(&self) -> &dyn Any {
        self
    }
    
    fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
    
    fn get_parameter(&self, name: &str) -> Option<String> {
        match name {
            "MassFlowRate" => Some(format!("{:.2}", self.mass_flow_rate)),
            "DensityFlow" => Some(format!("{:.4}", self.density_flow)),
            "Temperature" => Some(format!("{:.2}", self.temperature)),
            "VolumeFlowRate" => Some(format!("{:.3}", self.volume_flow_rate)),
            "MassTotal" => Some(format!("{:.2}", self.mass_total)),
            "VolumeTotal" => Some(format!("{:.3}", self.volume_total)),
            "MassInventory" => Some(format!("{:.2}", self.mass_inventory)),
            "VolumeInventory" => Some(format!("{:.3}", self.volume_inventory)),
            "ErrorCode" => Some(self.error_code.to_string()),
            _ => None,
        }
    }
    
    fn get_all_parameters(&self) -> Vec<(String, String)> {
        vec![
            ("ErrorCode".to_string(), self.error_code.to_string()),
            ("MassFlowRate".to_string(), format!("{:.2}", self.mass_flow_rate)),
            ("DensityFlow".to_string(), format!("{:.4}", self.density_flow)),
            ("Temperature".to_string(), format!("{:.2}", self.temperature)),
            ("VolumeFlowRate".to_string(), format!("{:.3}", self.volume_flow_rate)),
            ("MassTotal".to_string(), format!("{:.2}", self.mass_total)),
            ("VolumeTotal".to_string(), format!("{:.3}", self.volume_total)),
            ("MassInventory".to_string(), format!("{:.2}", self.mass_inventory)),
            ("VolumeInventory".to_string(), format!("{:.3}", self.volume_inventory)),
        ]
    }
    
    fn device_address(&self) -> u8 {
        self.device_address
    }
}