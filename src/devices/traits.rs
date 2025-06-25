use async_trait::async_trait;
use serde_json::Value;
use std::any::Any;

use crate::modbus::client::ModbusClientTrait;
use crate::utils::error::ModbusError;

#[async_trait]
pub trait Device: Send + Sync {
    fn device_type(&self) -> &str;
    fn address(&self) -> u8;
    fn name(&self) -> &str;
    fn as_any(&self) -> &dyn Any;

    async fn read_data(&self, client: &dyn ModbusClientTrait) -> Result<Box<dyn DeviceData>, ModbusError>;
    async fn reset_accumulation(&self, client: &dyn ModbusClientTrait) -> Result<(), ModbusError>;
    fn parse_raw_data(&self, data: &[u8]) -> Result<Box<dyn DeviceData>, ModbusError>;
}

pub trait DeviceData: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn to_json(&self) -> Value;
    fn get_parameter(&self, name: &str) -> Option<String>;
    fn get_all_parameters(&self) -> Vec<(String, String)>;
    fn device_address(&self) -> u8;

    fn timestamp(&self) -> chrono::DateTime<chrono::Utc>;
    fn device_type(&self) -> String {
        "unknown".to_string()
    }
    fn device_name(&self) -> String {
        format!("Device {}", self.device_address())
    }
    fn device_location(&self) -> String {
        "Unknown".to_string()
    }
}