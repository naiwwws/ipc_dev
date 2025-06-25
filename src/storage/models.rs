use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DeviceReading {
    pub id: Option<i64>,
    pub device_uuid: String,
    pub device_address: u8,
    pub device_type: String,
    pub device_name: String,
    pub device_location: String,
    pub parameter_name: String,
    pub parameter_value: String,
    pub parameter_unit: Option<String>,
    pub raw_value: Option<f64>,
    pub timestamp: DateTime<Utc>,
    pub ipc_uuid: String,
    pub site_id: String,
    pub batch_id: Option<String>,
    pub quality_flag: String, // "GOOD", "BAD", "UNCERTAIN"
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DeviceStatus {
    pub device_uuid: String,
    pub device_address: u8,
    pub last_seen: DateTime<Utc>,
    pub status: String, // "ONLINE", "OFFLINE", "ERROR"
    pub error_count: i32,
    pub total_readings: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SystemMetrics {
    pub id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub active_devices: i32,
    pub total_readings: i64,
    pub error_count: i32,
    pub uptime_seconds: i64,
    pub memory_usage_mb: f64,
    pub cpu_usage_percent: f64,
}

#[derive(Debug, Clone)]
pub struct BatchInsertData {
    pub readings: Vec<DeviceReading>,
    pub batch_id: String,
    pub created_at: DateTime<Utc>,
}

impl DeviceReading {
    pub fn new(
        device_uuid: String,
        device_address: u8,
        device_type: String,
        device_name: String,
        device_location: String,
        parameter_name: String,
        parameter_value: String,
        ipc_uuid: String,
        site_id: String,
    ) -> Self {
        Self {
            id: None,
            device_uuid,
            device_address,
            device_type,
            device_name,
            device_location,
            parameter_name,
            parameter_value: parameter_value.clone(), // Clone here to avoid move
            parameter_unit: None,
            raw_value: parameter_value.parse().ok(), // Use original here
            timestamp: Utc::now(),
            batch_id: None,
            ipc_uuid,
            site_id,
            quality_flag: "GOOD".to_string(), // Default value for quality_flag
        }
    }
}