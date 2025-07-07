use log::{info, error, warn, debug};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio::time::{interval, Duration};
use chrono::{DateTime, Utc};

use crate::config::Config;
use crate::devices::traits::DeviceData;
use crate::storage::{SqliteManager, models::{FlowmeterReading, FlowmeterStats}};
use crate::utils::error::ModbusError;

#[derive(Clone)] // FIX: Add Clone derive
pub struct DatabaseService {
    config: Config,
    sqlite_manager: SqliteManager,
    flowmeter_batch_buffer: Arc<RwLock<Vec<FlowmeterReading>>>,
    is_running: Arc<RwLock<bool>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl DatabaseService {
    pub async fn new(config: Config) -> Result<Self, ModbusError> {
        // FIX: Use the sqlite_config directly from database_output
        let database_config = config.output.database_output
            .as_ref()
            .ok_or_else(|| ModbusError::CommunicationError("Database output not configured".to_string()))?;
        
        // FIX: Use the existing sqlite_config instead of creating a new one
        let sqlite_config = database_config.sqlite_config.clone();
        
        let sqlite_manager = SqliteManager::new(sqlite_config).await?;
        
        Ok(Self {
            sqlite_manager,
            config,
            flowmeter_batch_buffer: Arc::new(RwLock::new(Vec::new())),
            is_running: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
        })
    }

    // MINIMAL: Store only essential device data
    pub async fn store_device_data(
        &self,
        _device_uuid: &str,  // Ignored
        device_address: u8,
        device_data: &dyn DeviceData,
    ) -> Result<(), ModbusError> {
        // Only process flowmeter data
        if let Some(flowmeter_data) = device_data.as_any().downcast_ref::<crate::devices::flowmeter::FlowmeterData>() {
            let reading = FlowmeterReading::from_flowmeter_data(device_address, flowmeter_data);
            self.add_flowmeter_to_batch(vec![reading]).await?;
        }
        Ok(())
    }

    // OPTIMIZED: Larger batching
    async fn add_flowmeter_to_batch(&self, readings: Vec<FlowmeterReading>) -> Result<(), ModbusError> {
        if readings.is_empty() {
            return Ok(());
        }

        let batch_size = self.config.output.database_output
            .as_ref()
            .map(|db| db.batch_size)
            .unwrap_or(500); // Increased from 100

        let should_flush = {
            let mut buffer = self.flowmeter_batch_buffer.write().await;
            buffer.extend(readings);
            buffer.len() >= batch_size
        };

        if should_flush {
            let readings_to_flush = {
                let mut buffer = self.flowmeter_batch_buffer.write().await;
                buffer.drain(..).collect::<Vec<_>>()
            };

            if !readings_to_flush.is_empty() {
                let manager = self.sqlite_manager.clone();
                tokio::spawn(async move {
                    if let Err(e) = manager.batch_insert_flowmeter_readings(readings_to_flush).await {
                        error!("Failed to flush batch: {}", e);
                    }
                });
            }
        }

        Ok(())
    }

    // Get recent flowmeter readings
    pub async fn get_recent_flowmeter_readings(&self, limit: i64) -> Result<Vec<FlowmeterReading>, ModbusError> {
        self.sqlite_manager.get_recent_flowmeter_readings(limit, 0).await
    }

    // Get flowmeter statistics
    pub async fn get_flowmeter_stats(&self) -> Result<FlowmeterStats, ModbusError> {
        self.sqlite_manager.get_flowmeter_stats().await
    }

    // Query flowmeter data by device
    pub async fn get_device_flowmeter_readings(
        &self,
        device_address: u8,
        hours_back: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<FlowmeterReading>, ModbusError> {
        let start_time = hours_back.map(|hours| (Utc::now() - chrono::Duration::hours(hours)).timestamp());
        
        self.sqlite_manager.get_device_flowmeter_readings(
            device_address,
            start_time,
            None,
            limit,
        ).await
    }

    // Force flush remaining buffer
    pub async fn flush_buffer(&self) -> Result<(), ModbusError> {
        let mut buffer = self.flowmeter_batch_buffer.write().await;
        if !buffer.is_empty() {
            let readings_to_flush = buffer.drain(..).collect::<Vec<_>>();
            drop(buffer);
            
            info!("🔄 Flushing {} remaining readings to database", readings_to_flush.len());
            self.sqlite_manager.batch_insert_flowmeter_readings(readings_to_flush).await?;
        }
        Ok(())
    }

    // Stop the service
    pub async fn stop(&self) -> Result<(), ModbusError> {
        let mut is_running = self.is_running.write().await;
        *is_running = false;
        drop(is_running);

        // Flush remaining data
        self.flush_buffer().await?;

        info!("🛑 Database service stopped");
        Ok(())
    }
}
