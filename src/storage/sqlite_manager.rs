use chrono::{DateTime, Utc};
use log::{info, warn, error, debug};
use sqlx::{Pool, Sqlite, SqlitePool, Row};
use std::path::Path;
use std::time::Duration;
use tokio::sync::RwLock;
use std::sync::Arc;

use crate::config::SqliteConfig;
use crate::storage::models::{DeviceReading, DeviceStatus, SystemMetrics};
use crate::utils::error::ModbusError;

#[derive(Clone)]
pub struct SqliteManager {
    pool: Pool<Sqlite>,
    config: SqliteConfig,
    connection_count: Arc<RwLock<usize>>,
}

impl SqliteManager {
    pub async fn new(config: SqliteConfig) -> Result<Self, ModbusError> {
        // Create database directory if it doesn't exist
        if let Some(parent) = Path::new(&config.database_path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ModbusError::CommunicationError(format!("Failed to create database directory: {}", e))
            })?;
        }

        // Build connection string with optimizations
        let connection_string = format!(
            "sqlite:{}?cache=shared&_busy_timeout={}&_journal_mode={}",
            config.database_path,
            config.busy_timeout_ms,
            if config.wal_mode { "WAL" } else { "DELETE" }
        );

        info!("🗄️  Initializing SQLite database: {}", config.database_path);

        // Create connection pool with optimized settings
        let pool = SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&config.database_path)
                .create_if_missing(true)
                .busy_timeout(Duration::from_millis(config.busy_timeout_ms as u64))
                .journal_mode(if config.wal_mode {
                    sqlx::sqlite::SqliteJournalMode::Wal
                } else {
                    sqlx::sqlite::SqliteJournalMode::Delete
                })
                .synchronous(match config.sync_mode.as_str() {
                    "OFF" => sqlx::sqlite::SqliteSynchronous::Off,
                    "NORMAL" => sqlx::sqlite::SqliteSynchronous::Normal,
                    "FULL" => sqlx::sqlite::SqliteSynchronous::Full,
                    _ => sqlx::sqlite::SqliteSynchronous::Normal,
                })
        ).await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to connect to SQLite: {}", e))
        })?;

        // Apply performance optimizations
        sqlx::query(&format!("PRAGMA cache_size = -{}", config.cache_size_kb))
            .execute(&pool)
            .await
            .map_err(|e| ModbusError::CommunicationError(format!("Failed to set cache size: {}", e)))?;

        if config.auto_vacuum {
            sqlx::query("PRAGMA auto_vacuum = INCREMENTAL")
                .execute(&pool)
                .await
                .map_err(|e| ModbusError::CommunicationError(format!("Failed to set auto_vacuum: {}", e)))?;
        }

        let manager = Self {
            pool,
            config,
            connection_count: Arc::new(RwLock::new(0)),
        };

        // Initialize database schema
        manager.initialize_schema().await?;

        info!("✅ SQLite database initialized successfully");
        Ok(manager)
    }

    async fn initialize_schema(&self) -> Result<(), ModbusError> {
        info!("🔧 Initializing database schema...");

        // Device readings table - optimized for time-series data
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS device_readings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_uuid TEXT NOT NULL,
                device_address INTEGER NOT NULL,
                device_type TEXT NOT NULL,
                device_name TEXT NOT NULL,
                device_location TEXT NOT NULL,
                parameter_name TEXT NOT NULL,
                parameter_value TEXT NOT NULL,
                parameter_unit TEXT,
                raw_value REAL,
                timestamp DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                ipc_uuid TEXT NOT NULL,
                site_id TEXT NOT NULL,
                batch_id TEXT,
                quality_flag TEXT NOT NULL DEFAULT 'GOOD'
            )
        "#).execute(&self.pool).await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to create device_readings table: {}", e))
        })?;

        // Device status table
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS device_status (
                device_uuid TEXT PRIMARY KEY,
                device_address INTEGER NOT NULL,
                last_seen DATETIME NOT NULL,
                status TEXT NOT NULL DEFAULT 'UNKNOWN',
                error_count INTEGER NOT NULL DEFAULT 0,
                total_readings INTEGER NOT NULL DEFAULT 0
            )
        "#).execute(&self.pool).await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to create device_status table: {}", e))
        })?;

        // System metrics table
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS system_metrics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                active_devices INTEGER NOT NULL,
                total_readings INTEGER NOT NULL,
                error_count INTEGER NOT NULL,
                uptime_seconds INTEGER NOT NULL,
                memory_usage_mb REAL,
                cpu_usage_percent REAL
            )
        "#).execute(&self.pool).await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to create system_metrics table: {}", e))
        })?;

        // Create optimized indexes for performance
        self.create_indexes().await?;

        info!("✅ Database schema initialized");
        Ok(())
    }

    async fn create_indexes(&self) -> Result<(), ModbusError> {
        let indexes = vec![
            // Primary query patterns
            "CREATE INDEX IF NOT EXISTS idx_readings_timestamp ON device_readings(timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_readings_device_uuid ON device_readings(device_uuid)",
            "CREATE INDEX IF NOT EXISTS idx_readings_device_addr ON device_readings(device_address)",
            "CREATE INDEX IF NOT EXISTS idx_readings_parameter ON device_readings(parameter_name)",
            "CREATE INDEX IF NOT EXISTS idx_readings_batch ON device_readings(batch_id)",
            
            // Composite indexes for common queries
            "CREATE INDEX IF NOT EXISTS idx_readings_device_time ON device_readings(device_uuid, timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_readings_addr_time ON device_readings(device_address, timestamp DESC)",
            "CREATE INDEX IF NOT EXISTS idx_readings_param_time ON device_readings(parameter_name, timestamp DESC)",
            
            // Status table indexes
            "CREATE INDEX IF NOT EXISTS idx_status_last_seen ON device_status(last_seen DESC)",
            "CREATE INDEX IF NOT EXISTS idx_status_address ON device_status(device_address)",
        ];

        for index_sql in indexes {
            sqlx::query(index_sql).execute(&self.pool).await.map_err(|e| {
                ModbusError::CommunicationError(format!("Failed to create index: {}", e))
            })?;
        }

        info!("✅ Database indexes created");
        Ok(())
    }

    // High-performance batch insert with transaction
    pub async fn batch_insert_readings(&self, readings: Vec<DeviceReading>) -> Result<usize, ModbusError> {
        if readings.is_empty() {
            return Ok(0);
        }

        let batch_size = self.config.batch_size;
        let total_count = readings.len();
        
        debug!("📦 Starting batch insert of {} readings", total_count);

        // Process in chunks to avoid memory issues
        let mut total_inserted = 0;
        for chunk in readings.chunks(batch_size) {
            let inserted = self.insert_readings_chunk(chunk).await?;
            total_inserted += inserted;
        }

        info!("✅ Batch insert completed: {} readings inserted", total_inserted);
        Ok(total_inserted)
    }

    async fn insert_readings_chunk(&self, readings: &[DeviceReading]) -> Result<usize, ModbusError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to start transaction: {}", e))
        })?;

        let mut inserted_count = 0;

        for reading in readings {
            let result = sqlx::query(r#"
                INSERT INTO device_readings (
                    device_uuid, device_address, device_type, device_name, device_location,
                    parameter_name, parameter_value, parameter_unit, raw_value, timestamp,
                    ipc_uuid, site_id, batch_id, quality_flag
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#)
            .bind(&reading.device_uuid)
            .bind(reading.device_address)
            .bind(&reading.device_type)
            .bind(&reading.device_name)
            .bind(&reading.device_location)
            .bind(&reading.parameter_name)
            .bind(&reading.parameter_value)
            .bind(&reading.parameter_unit)
            .bind(reading.raw_value)
            .bind(reading.timestamp)
            .bind(&reading.ipc_uuid)
            .bind(&reading.site_id)
            .bind(&reading.batch_id)
            .bind(&reading.quality_flag)
            .execute(&mut *tx)
            .await;

            match result {
                Ok(_) => inserted_count += 1,
                Err(e) => {
                    warn!("Failed to insert reading: {}", e);
                    // Continue with other readings instead of failing entire batch
                }
            }
        }

        tx.commit().await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to commit transaction: {}", e))
        })?;

        Ok(inserted_count)
    }

    // Update device status efficiently
    pub async fn update_device_status(&self, device_uuid: &str, device_address: u8, status: &str) -> Result<(), ModbusError> {
        sqlx::query(r#"
            INSERT OR REPLACE INTO device_status (
                device_uuid, device_address, last_seen, status, 
                error_count, total_readings
            ) VALUES (
                ?, ?, CURRENT_TIMESTAMP, ?,
                COALESCE((SELECT error_count FROM device_status WHERE device_uuid = ?), 0),
                COALESCE((SELECT total_readings FROM device_status WHERE device_uuid = ?), 0) + 1
            )
        "#)
        .bind(device_uuid)
        .bind(device_address)
        .bind(status)
        .bind(device_uuid)
        .bind(device_uuid)
        .execute(&self.pool)
        .await
        .map_err(|e| ModbusError::CommunicationError(format!("Failed to update device status: {}", e)))?;

        Ok(())
    }

    // Get recent readings with pagination
    pub async fn get_recent_readings(&self, limit: i64, offset: i64) -> Result<Vec<DeviceReading>, ModbusError> {
        let readings = sqlx::query_as::<_, DeviceReading>(r#"
            SELECT * FROM device_readings 
            ORDER BY timestamp DESC 
            LIMIT ? OFFSET ?
        "#)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ModbusError::CommunicationError(format!("Failed to fetch recent readings: {}", e)))?;

        Ok(readings)
    }

    // Get readings by device with time range
    pub async fn get_device_readings(
        &self, 
        device_uuid: &str, 
        start_time: Option<DateTime<Utc>>, 
        end_time: Option<DateTime<Utc>>,
        limit: Option<i64>
    ) -> Result<Vec<DeviceReading>, ModbusError> {
        let mut query = "SELECT * FROM device_readings WHERE device_uuid = ?".to_string();
        let mut bindings = vec![device_uuid.to_string()];

        if let Some(start) = start_time {
            query.push_str(" AND timestamp >= ?");
            bindings.push(start.to_rfc3339());
        }

        if let Some(end) = end_time {
            query.push_str(" AND timestamp <= ?");
            bindings.push(end.to_rfc3339());
        }

        query.push_str(" ORDER BY timestamp DESC");

        if let Some(limit_val) = limit {
            query.push_str(" LIMIT ?");
            bindings.push(limit_val.to_string());
        }

        let mut query_builder = sqlx::query_as::<_, DeviceReading>(&query);
        for binding in bindings {
            query_builder = query_builder.bind(binding);
        }

        let readings = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ModbusError::CommunicationError(format!("Failed to fetch device readings: {}", e)))?;

        Ok(readings)
    }

    // Database maintenance operations
    pub async fn cleanup_old_data(&self, days_to_keep: i64) -> Result<u64, ModbusError> {
        let cutoff_date = Utc::now() - chrono::Duration::days(days_to_keep);
        
        let result = sqlx::query("DELETE FROM device_readings WHERE timestamp < ?")
            .bind(cutoff_date)
            .execute(&self.pool)
            .await
            .map_err(|e| ModbusError::CommunicationError(format!("Failed to cleanup old data: {}", e)))?;

        // Run VACUUM to reclaim space
        sqlx::query("VACUUM")
            .execute(&self.pool)
            .await
            .map_err(|e| ModbusError::CommunicationError(format!("Failed to vacuum database: {}", e)))?;

        info!("🧹 Cleaned up {} old records", result.rows_affected());
        Ok(result.rows_affected())
    }

    // Get database statistics
    pub async fn get_database_stats(&self) -> Result<DatabaseStats, ModbusError> {
        let total_readings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_readings")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| ModbusError::CommunicationError(format!("Failed to get reading count: {}", e)))?;

        let active_devices: i64 = sqlx::query_scalar(r#"
            SELECT COUNT(DISTINCT device_uuid) FROM device_readings 
            WHERE timestamp > datetime('now', '-1 hour')
        "#)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ModbusError::CommunicationError(format!("Failed to get active device count: {}", e)))?;

        let db_size = std::fs::metadata(&self.config.database_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);

        Ok(DatabaseStats {
            total_readings,
            active_devices,
            database_size_bytes: db_size,
            connection_count: *self.connection_count.read().await,
        })
    }

    // Close all connections gracefully
    pub async fn close(&self) {
        info!("🔒 Closing SQLite database connections");
        self.pool.close().await;
    }
}

#[derive(Debug)]
pub struct DatabaseStats {
    pub total_readings: i64,
    pub active_devices: i64,
    pub database_size_bytes: u64,
    pub connection_count: usize,
}