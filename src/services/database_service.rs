use chrono::{DateTime, Utc};
use log::{info, warn, error, debug};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Duration};

use crate::config::Config;
use crate::devices::DeviceData;
use crate::storage::{SqliteManager, FlowmeterReading, FlowmeterStats, DatabaseStats}; // ✅ Add DatabaseStats import
use crate::utils::error::ModbusError;

pub struct DatabaseService {
    config: Config,
    sqlite_manager: SqliteManager,
    flowmeter_batch_buffer: Arc<RwLock<Vec<FlowmeterReading>>>,
    is_running: Arc<RwLock<bool>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl DatabaseService {
    pub async fn new(config: Config) -> Result<Self, ModbusError> {
        let sqlite_config = config.output.database_output
            .as_ref()
            .map(|db_config| db_config.sqlite_config.clone())
            .unwrap_or_default();

        let sqlite_manager = SqliteManager::new(sqlite_config).await?;
        
        Ok(Self {
            config,
            sqlite_manager,
            flowmeter_batch_buffer: Arc::new(RwLock::new(Vec::new())),
            is_running: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
        })
    }

    // ✅ Store flowmeter data as unified structure
    pub async fn store_device_data(
        &self,
        device_uuid: &str,
        device_address: u8,
        device_type: &str,
        device_name: &str,
        device_location: &str,
        device_data: &dyn DeviceData,
    ) -> Result<(), ModbusError> {
        // Only support flowmeter data with unified storage
        if device_type.to_lowercase() == "flowmeter" {
            if let Some(flowmeter_data) = device_data.as_any().downcast_ref::<crate::devices::FlowmeterData>() {
                let flowmeter_reading = FlowmeterReading::from_flowmeter_data(
                    device_uuid.to_string(),
                    device_address,
                    device_name.to_string(),
                    device_location.to_string(),
                    flowmeter_data,
                    self.config.get_ipc_uuid().to_string(),
                    self.config.site_info.site_id.clone(),
                );

                self.add_flowmeter_to_batch(vec![flowmeter_reading]).await?;
                self.sqlite_manager.update_device_status(device_uuid, device_address, "ONLINE").await?;
                
                return Ok(());
            }
        }

        // Reject non-flowmeter devices
        warn!("⚠️  Only flowmeter devices are supported in the unified storage system");
        Ok(())
    }

    // ✅ Add flowmeter readings to batch
    async fn add_flowmeter_to_batch(&self, readings: Vec<FlowmeterReading>) -> Result<(), ModbusError> {
        if readings.is_empty() {
            return Ok(());
        }

        let batch_size = self.config.output.database_output
            .as_ref()
            .map(|db| db.batch_size)
            .unwrap_or(100);

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
                        error!("Failed to flush flowmeter batch to database: {}", e);
                    }
                });
            }
        }

        Ok(())
    }

    // ✅ Start batch processor for flowmeter data
    pub async fn start(&mut self) -> Result<(), ModbusError> {
        if *self.is_running.read().await {
            return Ok(());
        }

        *self.is_running.write().await = true;

        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Start flowmeter batch processor
        let buffer = self.flowmeter_batch_buffer.clone();
        let manager = self.sqlite_manager.clone();
        let config = self.config.clone();
        let is_running = self.is_running.clone();

        tokio::spawn(async move {
            Self::flowmeter_batch_processor(buffer, manager, config, is_running, shutdown_rx).await;
        });

        info!("🚀 Flowmeter database service started");
        Ok(())
    }

    // ✅ Flowmeter batch processor
    async fn flowmeter_batch_processor(
        buffer: Arc<RwLock<Vec<FlowmeterReading>>>,
        manager: SqliteManager,
        config: Config,
        is_running: Arc<RwLock<bool>>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        let flush_interval = Duration::from_secs(
            config.output.database_output
                .as_ref()
                .map(|db| db.flush_interval_seconds)
                .unwrap_or(60)
        );

        let mut interval = interval(flush_interval);

        info!("🔄 Flowmeter batch processor started with {}s intervals", flush_interval.as_secs());

        while *is_running.read().await {
            tokio::select! {
                _ = interval.tick() => {
                    let readings_to_flush = {
                        let mut buffer_guard = buffer.write().await;
                        if !buffer_guard.is_empty() {
                            buffer_guard.drain(..).collect::<Vec<_>>()
                        } else {
                            continue;
                        }
                    };

                    if !readings_to_flush.is_empty() {
                        debug!("⏰ Periodic flush: {} flowmeter readings", readings_to_flush.len());
                        if let Err(e) = manager.batch_insert_flowmeter_readings(readings_to_flush).await {
                            error!("Failed to flush flowmeter batch: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("📥 Received shutdown signal for flowmeter batch processor");
                    break;
                }
            }
        }

        // Final flush on shutdown
        let final_readings = {
            let mut buffer_guard = buffer.write().await;
            buffer_guard.drain(..).collect::<Vec<_>>()
        };

        if !final_readings.is_empty() {
            info!("🔄 Final flowmeter flush: {} readings", final_readings.len());
            if let Err(e) = manager.batch_insert_flowmeter_readings(final_readings).await {
                error!("Failed to flush final flowmeter batch: {}", e);
            }
        }

        info!("🏁 Flowmeter batch processor stopped");
    }

    // ✅ Get flowmeter readings
    pub async fn get_recent_flowmeter_readings(&self, limit: i64) -> Result<Vec<FlowmeterReading>, ModbusError> {
        self.sqlite_manager.get_recent_flowmeter_readings(limit, 0).await
    }

    // ✅ Get device flowmeter readings
    pub async fn get_device_flowmeter_readings(
        &self,
        device_uuid: &str,
        hours_back: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<FlowmeterReading>, ModbusError> {
        let start_time = hours_back.map(|hours| Utc::now() - chrono::Duration::hours(hours));
        
        self.sqlite_manager.get_device_flowmeter_readings(
            device_uuid,
            start_time,
            None,
            limit,
        ).await
    }

    // ✅ Get flowmeter statistics
    pub async fn get_flowmeter_stats(&self) -> Result<FlowmeterStats, ModbusError> {
        self.sqlite_manager.get_flowmeter_stats().await
    }

    // ✅ Force flush flowmeter buffer
    pub async fn flush_flowmeter_buffer(&self) -> Result<usize, ModbusError> {
        let readings_to_flush = {
            let mut buffer = self.flowmeter_batch_buffer.write().await;
            if buffer.is_empty() {
                return Ok(0);
            }
            buffer.drain(..).collect::<Vec<_>>()
        };

        let count = readings_to_flush.len();
        self.sqlite_manager.batch_insert_flowmeter_readings(readings_to_flush).await?;
        info!("🔄 Manual flowmeter flush: {} readings inserted", count);
        Ok(count)
    }

    // ✅ Graceful shutdown
    pub async fn shutdown(&mut self) -> Result<(), ModbusError> {
        info!("🛑 Shutting down database service...");
        
        *self.is_running.write().await = false;
        
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        
        // Force final flush
        let _ = self.flush_flowmeter_buffer().await;
        
        info!("✅ Database service shutdown complete");
        Ok(())
    }

    // ✅ Add the missing get_stats method
    pub async fn get_stats(&self) -> Result<DatabaseStats, ModbusError> {
        self.sqlite_manager.get_database_stats().await
    }
}

impl Drop for DatabaseService {
    fn drop(&mut self) {
        info!("💧 DatabaseService dropped");
    }
}
