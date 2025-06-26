use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

//  CORRECTED: Unified Flowmeter Reading Structure
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FlowmeterReading {
    pub id: Option<i64>,
    pub device_uuid: String,
    pub device_address: u8,
    pub device_name: String,
    pub device_location: String,
    pub timestamp: DateTime<Utc>,

    // Flow measurement parameters
    pub mass_flow_rate: f32,
    pub mass_flow_rate_unit: String,
    pub density_flow: f32,
    pub density_flow_unit: String,
    pub temperature: f32,
    pub temperature_unit: String,
    pub volume_flow_rate: f32,
    pub volume_flow_rate_unit: String,

    // Accumulation parameters
    pub mass_total: f32,
    pub mass_total_unit: String,
    pub volume_total: f32,
    pub volume_total_unit: String,
    pub mass_inventory: f32,
    pub mass_inventory_unit: String,
    pub volume_inventory: f32,
    pub volume_inventory_unit: String,

    // System status
    pub error_code: u16,

    // Metadata
    pub ipc_uuid: String,
    pub site_id: String,
    pub batch_id: Option<String>,
    pub quality_flag: String,
    pub created_at: DateTime<Utc>,
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
}

// Keep SystemMetrics for system monitoring
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

//  CORRECTED: Statistics for flowmeter data
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FlowmeterStats {
    pub total_readings: i64,
    pub active_devices: i64,
    pub avg_mass_flow_rate: Option<f64>,
    pub max_mass_flow_rate: Option<f64>,
    pub min_mass_flow_rate: Option<f64>,
    pub avg_temperature: Option<f64>,
    pub latest_reading: Option<DateTime<Utc>>,
    pub earliest_reading: Option<DateTime<Utc>>,
}

//  CORRECTED: Constructor for FlowmeterReading
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
        Self {
            id: None,
            device_uuid,
            device_address,
            device_name,
            device_location,
            timestamp: flowmeter_data.timestamp,

            // Flow parameters with units
            mass_flow_rate: flowmeter_data.mass_flow_rate,
            mass_flow_rate_unit: "kg/h".to_string(),
            density_flow: flowmeter_data.density_flow,
            density_flow_unit: "kg/L".to_string(),
            temperature: flowmeter_data.temperature,
            temperature_unit: "°C".to_string(),
            volume_flow_rate: flowmeter_data.volume_flow_rate,
            volume_flow_rate_unit: "L/h".to_string(),

            // Accumulation parameters
            mass_total: flowmeter_data.mass_total,
            mass_total_unit: "kg".to_string(),
            volume_total: flowmeter_data.volume_total,
            volume_total_unit: "L".to_string(),
            mass_inventory: flowmeter_data.mass_inventory,
            mass_inventory_unit: "kg".to_string(),
            volume_inventory: flowmeter_data.volume_inventory,
            volume_inventory_unit: "L".to_string(),

            // Status
            error_code: flowmeter_data.error_code,

            // System fields
            ipc_uuid,
            site_id,
            batch_id: None,
            quality_flag: "GOOD".to_string(),
            created_at: Utc::now(),
        }
    }
}