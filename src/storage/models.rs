use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::devices::traits::DeviceData;

// MINIMAL: Essential flowmeter reading structure
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FlowmeterReading {
    pub id: Option<i64>,
    pub device_address: u8,
    pub unix_timestamp: i64,  // Changed from unix_ts
    
    // Core measurement data
    pub mass_flow_rate: f32,
    pub density_flow: f32,
    pub temperature: f32,
    pub volume_flow_rate: f32,
    pub mass_total: f32,
    pub volume_total: f32,
    pub error_code: u16,
}

// Keep minimal stats structure
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FlowmeterStats {
    pub total_readings: i64,
    pub avg_mass_flow_rate: Option<f32>,
    pub max_mass_flow_rate: Option<f32>,
    pub min_mass_flow_rate: Option<f32>,
    pub avg_temperature: Option<f32>,
    pub latest_timestamp: Option<i64>,
    pub earliest_timestamp: Option<i64>,
}

// MINIMAL: Constructor for FlowmeterReading
impl FlowmeterReading {
    pub fn from_flowmeter_data(
        device_address: u8,
        flowmeter_data: &crate::devices::flowmeter::FlowmeterData,
    ) -> Self {
        Self {
            id: None,
            device_address,
            unix_timestamp: flowmeter_data.unix_ts(), // Fixed method name
            mass_flow_rate: flowmeter_data.mass_flow_rate,
            density_flow: flowmeter_data.density_flow,
            temperature: flowmeter_data.temperature,
            volume_flow_rate: flowmeter_data.volume_flow_rate,
            mass_total: flowmeter_data.mass_total,
            volume_total: flowmeter_data.volume_total,
            error_code: flowmeter_data.error_code,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Transaction {
    pub id: Option<i64>,
    pub transaction_id: String,
    pub flow_type: String,
    pub vessel_id: String,
    pub vessel_name: String,
    pub vessel_type: String,
    pub liquid_target_volume: f32,
    pub liquid_type: String,
    pub liquid_density_min: f32,
    pub liquid_density_max: f32,
    pub liquid_water_content_min: f32,
    pub liquid_water_content_max: f32,
    pub liquid_residual_carbon_min: f32,
    pub liquid_residual_carbon_max: f32,
    pub operator_full_name: String,
    pub operator_email: String,
    pub operator_phone_number: Option<String>,
    pub customer_vessel_name: Option<String>,
    pub customer_pic_name: Option<String>,
    pub customer_location_name: Option<String>,
    pub supplier_name: Option<String>,
    pub created_at: i64, // Unix timestamp
    pub status: String,
}

impl Transaction {
    pub fn new(
        transaction_id: String,
        flow_type: String,
        vessel_id: String,
        vessel_name: String,
        vessel_type: String,
        liquid_target_volume: f32,
        liquid_type: String,
        liquid_density_min: f32,
        liquid_density_max: f32,
        liquid_water_content_min: f32,
        liquid_water_content_max: f32,
        liquid_residual_carbon_min: f32,
        liquid_residual_carbon_max: f32,
        operator_full_name: String,
        operator_email: String,
        operator_phone_number: Option<String>,
        customer_vessel_name: Option<String>,
        customer_pic_name: Option<String>,
        customer_location_name: Option<String>,
        supplier_name: Option<String>,
        status: String,
    ) -> Self {
        Self {
            id: None,
            transaction_id,
            flow_type,
            vessel_id,
            vessel_name,
            vessel_type,
            liquid_target_volume,
            liquid_type,
            liquid_density_min,
            liquid_density_max,
            liquid_water_content_min,
            liquid_water_content_max,
            liquid_residual_carbon_min,
            liquid_residual_carbon_max,
            operator_full_name,
            operator_email,
            operator_phone_number,
            customer_vessel_name,
            customer_pic_name,
            customer_location_name,
            supplier_name,
            created_at: Utc::now().timestamp(),
            status,
        }
    }
}