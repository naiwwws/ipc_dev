use chrono::{DateTime, Utc};
use log::{info, warn, error, debug};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Duration};
use uuid::Uuid;

use crate::config::{Config, SqliteConfig};
use crate::devices::DeviceData;
use crate::storage::{SqliteManager, DeviceReading};
use crate::utils::error::ModbusError;

pub struct DatabaseService {
    sqlite_manager: SqliteManager,
    config: Config,
    batch_buffer: Arc<RwLock<Vec<DeviceReading>>>,
    is_running: Arc<RwLock<bool>>,
    // Channel for graceful shutdown
    shutdown_tx: Option<mpsc::Sender<()>>,
    shutdown_rx: Option<mpsc::Receiver<()>>,
}

impl DatabaseService {
    pub async fn new(config: Config) -> Result<Self, ModbusError> {
        let sqlite_config = config.output.database_output
            .as_ref()
            .map(|db_config| db_config.sqlite_config.clone())
            .unwrap_or_default();

        let sqlite_manager = SqliteManager::new(sqlite_config).await?;
        
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        Ok(Self {
            sqlite_manager,
            config,
            batch_buffer: Arc::new(RwLock::new(Vec::new())),
            is_running: Arc::new(RwLock::new(false)),
            shutdown_tx: Some(shutdown_tx),
            shutdown_rx: Some(shutdown_rx),
        })
    }

    // Start the database service with background tasks
    pub async fn start(&mut self) -> Result<(), ModbusError> {
        info!("🚀 Starting Database Service");
        
        *self.is_running.write().await = true;
        
        // Start background batch processor
        let buffer_clone = Arc::clone(&self.batch_buffer);
        let manager_clone = self.sqlite_manager.clone();
        let config_clone = self.config.clone();
        let running_clone = Arc::clone(&self.is_running);
        let mut shutdown_rx = self.shutdown_rx.take().unwrap();

        tokio::spawn(async move {
            Self::batch_processor(
                buffer_clone,
                manager_clone,
                config_clone,
                running_clone,
                shutdown_rx,
            ).await;
        });

        // Start maintenance tasks
        self.start_maintenance_tasks().await;
        
        info!("✅ Database Service started successfully");
        Ok(())
    }

    // Store device data efficiently
    pub async fn store_device_data(
        &self,
        device_uuid: &str,
        device_address: u8,
        device_type: &str,
        device_name: &str,
        device_location: &str,
        device_data: &dyn DeviceData,
    ) -> Result<(), ModbusError> {
        let parameters = device_data.get_all_parameters();
        let mut readings = Vec::new();

        // Convert device parameters to database readings
        for (param_name, param_value) in parameters {
            let reading = DeviceReading::new(
                device_uuid.to_string(),
                device_address,
                device_type.to_string(),
                device_name.to_string(),
                device_location.to_string(),
                param_name,
                param_value,
                self.config.get_ipc_uuid().to_string(),
                self.config.site_info.site_id.clone(),
            );
            readings.push(reading);
        }

        // Add to batch buffer
        self.add_to_batch(readings).await?;
        
        // Update device status
        self.sqlite_manager.update_device_status(device_uuid, device_address, "ONLINE").await?;

        Ok(())
    }

    // Add readings to batch buffer
    async fn add_to_batch(&self, mut readings: Vec<DeviceReading>) -> Result<(), ModbusError> {
        let batch_id = Uuid::new_v4().to_string();
        
        // Set batch ID for all readings
        for reading in &mut readings {
            reading.batch_id = Some(batch_id.clone());
        }

        let mut buffer = self.batch_buffer.write().await;
        buffer.extend(readings);

        let batch_size = self.config.output.database_output
            .as_ref()
            .map(|db| db.sqlite_config.batch_size)
            .unwrap_or(100);

        // Trigger immediate flush if buffer is full
        if buffer.len() >= batch_size {
            let readings_to_flush = buffer.drain(..).collect();
            drop(buffer); // Release lock early

            // Spawn background task for insertion
            let manager_clone = self.sqlite_manager.clone();
            tokio::spawn(async move {
                if let Err(e) = manager_clone.batch_insert_readings(readings_to_flush).await {
                    error!("Failed to flush batch to database: {}", e);
                }
            });
        }

        Ok(())
    }

    // Background batch processor
    async fn batch_processor(
        buffer: Arc<RwLock<Vec<DeviceReading>>>,
        manager: SqliteManager,
        config: Config,
        is_running: Arc<RwLock<bool>>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        let flush_interval = Duration::from_secs(
            config.output.database_output
                .as_ref()
                .and_then(|db| Some(30)) // 30 seconds default flush interval
                .unwrap_or(30)
        );

        let mut interval_timer = interval(flush_interval);

        loop {
            tokio::select! {
                _ = interval_timer.tick() => {
                    if !*is_running.read().await {
                        break;
                    }
                    
                    // Flush buffer periodically
                    let readings_to_flush = {
                        let mut buffer_guard = buffer.write().await;
                        if buffer_guard.is_empty() {
                            continue;
                        }
                        buffer_guard.drain(..).collect::<Vec<_>>()
                    };

                    if !readings_to_flush.is_empty() {
                        debug!("⏰ Periodic flush: {} readings", readings_to_flush.len());
                        if let Err(e) = manager.batch_insert_readings(readings_to_flush).await {
                            error!("Failed to flush periodic batch: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("📥 Received shutdown signal for batch processor");
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
            info!("🔄 Final flush: {} readings", final_readings.len());
            if let Err(e) = manager.batch_insert_readings(final_readings).await {
                error!("Failed to flush final batch: {}", e);
            }
        }

        info!("🏁 Batch processor stopped");
    }

    // Start maintenance tasks
    async fn start_maintenance_tasks(&self) {
        let manager_clone = self.sqlite_manager.clone();
        let running_clone = Arc::clone(&self.is_running);

        // Database cleanup task (runs daily)
        tokio::spawn(async move {
            let mut cleanup_interval = interval(Duration::from_secs(24 * 60 * 60)); // 24 hours
            
            loop {
                cleanup_interval.tick().await;
                
                if !*running_clone.read().await {
                    break;
                }

                info!("🧹 Running daily database maintenance");
                
                // Keep 30 days of data by default
                if let Err(e) = manager_clone.cleanup_old_data(30).await {
                    warn!("Database cleanup failed: {}", e);
                } else {
                    info!("✅ Database cleanup completed");
                }
            }
        });
    }

    // Get database statistics
    pub async fn get_stats(&self) -> Result<crate::storage::sqlite_manager::DatabaseStats, ModbusError> {
        self.sqlite_manager.get_database_stats().await
    }

    // Get recent readings for CLI/monitoring
    pub async fn get_recent_readings(&self, limit: i64) -> Result<Vec<DeviceReading>, ModbusError> {
        self.sqlite_manager.get_recent_readings(limit, 0).await
    }

    // Get readings for specific device
    pub async fn get_device_readings(
        &self,
        device_uuid: &str,
        hours_back: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<DeviceReading>, ModbusError> {
        let start_time = hours_back.map(|hours| Utc::now() - chrono::Duration::hours(hours));
        
        self.sqlite_manager.get_device_readings(
            device_uuid,
            start_time,
            None,
            limit,
        ).await
    }

    // Force flush buffer (useful for testing or manual operations)
    pub async fn flush_buffer(&self) -> Result<usize, ModbusError> {
        let readings_to_flush = {
            let mut buffer = self.batch_buffer.write().await;
            buffer.drain(..).collect::<Vec<_>>()
        };

        let count = readings_to_flush.len();
        if count > 0 {
            self.sqlite_manager.batch_insert_readings(readings_to_flush).await?;
            info!("🔄 Manual flush completed: {} readings", count);
        }

        Ok(count)
    }

    // Graceful shutdown
    pub async fn shutdown(&mut self) -> Result<(), ModbusError> {
        info!("🛑 Shutting down Database Service");
        
        *self.is_running.write().await = false;
        
        // Signal shutdown to background tasks
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }

        // Flush remaining data
        self.flush_buffer().await?;
        
        // Close database connections
        self.sqlite_manager.close().await;
        
        info!("✅ Database Service shutdown completed");
        Ok(())
    }
}

// Implement Drop for cleanup
impl Drop for DatabaseService {
    fn drop(&mut self) {
        info!("💧 DatabaseService dropped");
    }
}