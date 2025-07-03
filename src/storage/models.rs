use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// FIX: Import DeviceData from the correct module path
use crate::devices::traits::DeviceData;

// CORRECTED: Unified Flowmeter Reading Structure with Unix timestamps
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FlowmeterReading {
    pub id: Option<i64>,
    pub device_uuid: String,
    pub device_address: u8,
    pub device_name: String,
    pub device_location: String,
    pub unix_timestamp: i64, // Unix timestamp for compatibility

    // Flow measurement parameters (units removed)
    pub mass_flow_rate: f32,
    pub density_flow: f32,
    pub temperature: f32,
    pub volume_flow_rate: f32,

    // Accumulation parameters (units removed)
    pub mass_total: f32,
    pub volume_total: f32,
    pub mass_inventory: f32,
    pub volume_inventory: f32,

    // System status
    pub error_code: u16,

    // Metadata
    pub ipc_uuid: String,
    pub site_id: String,
    pub batch_id: Option<String>,
    pub quality_flag: String,
    pub created_at: i64,
}

// Keep DeviceStatus for monitoring
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DeviceStatus {
    pub device_uuid: String,
    pub device_address: u8,
    pub last_seen: DateTime<Utc>,
    pub status: String,
    pub error_count: i32,
    pub total_readings: i64,
    pub updated_at: DateTime<Utc>,
}

// Keep SystemMetrics for system monitoring
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SystemMetrics {
    pub id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub disk_usage: f64,
    pub network_throughput: f64,
    pub active_connections: i32,
}

// CORRECTED: Statistics for flowmeter data
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FlowmeterStats {
    pub total_readings: i64,
    pub active_devices: i64,
    pub avg_mass_flow_rate: Option<f32>,
    pub max_mass_flow_rate: Option<f32>,
    pub min_mass_flow_rate: Option<f32>,
    pub avg_temperature: Option<f32>,
    pub latest_reading: Option<DateTime<Utc>>,
    pub earliest_reading: Option<DateTime<Utc>>,
}

// CORRECTED: Constructor for FlowmeterReading
impl FlowmeterReading {
    pub fn from_flowmeter_data(
        device_uuid: String,
        device_address: u8,
        device_name: String,
        device_location: String,
        flowmeter_data: &crate::devices::FlowmeterData,
        ipc_uuid: String,
        site_id: String,
    ) -> Self {
        let now = Utc::now().timestamp();
        
        Self {
            id: None,
            device_uuid,
            device_address,
            device_name,
            device_location,
            unix_timestamp: flowmeter_data.unix_timestamp(),
            mass_flow_rate: flowmeter_data.mass_flow_rate,
            density_flow: flowmeter_data.density_flow,
            temperature: flowmeter_data.temperature,
            volume_flow_rate: flowmeter_data.volume_flow_rate,
            mass_total: flowmeter_data.mass_total,
            volume_total: flowmeter_data.volume_total,
            mass_inventory: flowmeter_data.mass_inventory,
            volume_inventory: flowmeter_data.volume_inventory,
            error_code: flowmeter_data.error_code,
            ipc_uuid,
            site_id,
            batch_id: None,
            quality_flag: "GOOD".to_string(),
            created_at: now,
        }
    }
}