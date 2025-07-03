use chrono::{DateTime, Utc};
use log::{info, warn};
use sqlx::{Pool, Sqlite, SqlitePool};
use std::path::Path;
use std::time::Duration;
use tokio::sync::RwLock;
use std::sync::Arc;

use crate::config::settings::SqliteConfig;
use crate::storage::models::{FlowmeterReading, FlowmeterStats};
use crate::utils::error::ModbusError;

#[derive(Clone)]
pub struct SqliteManager {
    pool: SqlitePool,
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
            if config.enable_wal { "WAL" } else { "DELETE" }
        );

        info!("🗄️  Initializing SQLite database: {}", config.database_path);

        // Create connection pool with optimized settings
        let pool = SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&config.database_path)
                .create_if_missing(true)
                .busy_timeout(Duration::from_millis(config.busy_timeout_ms))
                .journal_mode(if config.enable_wal {
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
        sqlx::query(&format!("PRAGMA cache_size = -{}", config.cache_size))
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
                unix_timestamp INTEGER NOT NULL,
                ipc_uuid TEXT NOT NULL,
                site_id TEXT NOT NULL,
                batch_id TEXT,
                quality_flag TEXT NOT NULL DEFAULT 'GOOD'
            )
        "#).execute(&self.pool).await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to create device_readings table: {}", e))
        })?;

        // Create unified flowmeter_readings table with Unix timestamps
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS flowmeter_readings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_uuid TEXT NOT NULL,
                device_address INTEGER NOT NULL,
                device_name TEXT NOT NULL,
                device_location TEXT NOT NULL,
                unix_timestamp INTEGER NOT NULL,
                
                -- Flow measurement parameters (units removed)
                mass_flow_rate REAL NOT NULL,
                density_flow REAL NOT NULL,
                temperature REAL NOT NULL,
                volume_flow_rate REAL NOT NULL,
                
                -- Accumulation parameters (units removed)
                mass_total REAL NOT NULL,
                volume_total REAL NOT NULL,
                mass_inventory REAL NOT NULL,
                volume_inventory REAL NOT NULL,
                
                -- System status
                error_code INTEGER NOT NULL DEFAULT 0,
                
                -- Metadata
                ipc_uuid TEXT NOT NULL,
                site_id TEXT NOT NULL,
                batch_id TEXT,
                quality_flag TEXT NOT NULL DEFAULT 'GOOD',
                created_at INTEGER NOT NULL
            )
        "#)
        .execute(&self.pool)
        .await
        .map_err(|e| ModbusError::CommunicationError(format!("Failed to create flowmeter_readings table: {}", e)))?;

        // Device status table
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS device_status (
                device_uuid TEXT PRIMARY KEY,
                device_address INTEGER NOT NULL,
                last_seen INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'UNKNOWN',
                error_count INTEGER DEFAULT 0,
                total_readings INTEGER DEFAULT 0,
                updated_at INTEGER NOT NULL
            )
        "#)
        .execute(&self.pool)
        .await
        .map_err(|e| ModbusError::CommunicationError(format!("Failed to create device_status table: {}", e)))?;

        // Create indexes after schema
        self.create_indexes().await?;

        info!("✅ Database schema initialized successfully");
        Ok(())
    }

    async fn create_indexes(&self) -> Result<(), ModbusError> {
        let indexes = vec![
            // Flowmeter table indexes
            "CREATE INDEX IF NOT EXISTS idx_flowmeter_timestamp ON flowmeter_readings(unix_timestamp)",
            "CREATE INDEX IF NOT EXISTS idx_flowmeter_device_uuid ON flowmeter_readings(device_uuid)",
            "CREATE INDEX IF NOT EXISTS idx_flowmeter_device_address ON flowmeter_readings(device_address)",
            "CREATE INDEX IF NOT EXISTS idx_flowmeter_created_at ON flowmeter_readings(created_at)",
            
            // Composite indexes for common queries
            "CREATE INDEX IF NOT EXISTS idx_flowmeter_device_time ON flowmeter_readings(device_uuid, unix_timestamp)",
            "CREATE INDEX IF NOT EXISTS idx_flowmeter_address_time ON flowmeter_readings(device_address, unix_timestamp)",
            
            // Performance indexes
            "CREATE INDEX IF NOT EXISTS idx_flowmeter_quality ON flowmeter_readings(quality_flag)",
            "CREATE INDEX IF NOT EXISTS idx_flowmeter_error ON flowmeter_readings(error_code)",
        ];

        for index_sql in indexes {
            sqlx::query(index_sql)
                .execute(&self.pool)
                .await
                .map_err(|e| ModbusError::CommunicationError(format!("Failed to create index: {}", e)))?;
        }

        info!("✅ Database indexes created successfully");
        Ok(())
    }

    // Update device status efficiently
    pub async fn update_device_status(&self, device_uuid: &str, device_address: u8, status: &str) -> Result<(), ModbusError> {
        let now = Utc::now().timestamp();
        
        sqlx::query(r#"
            INSERT OR REPLACE INTO device_status (
                device_uuid, device_address, last_seen, status, 
                error_count, total_readings, updated_at
            ) VALUES (
                ?, ?, ?, ?,
                COALESCE((SELECT error_count FROM device_status WHERE device_uuid = ?), 0),
                COALESCE((SELECT total_readings FROM device_status WHERE device_uuid = ?), 0) + 1,
                ?
            )
        "#)
        .bind(device_uuid)
        .bind(device_address)
        .bind(now)
        .bind(status)
        .bind(device_uuid)
        .bind(device_uuid)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| ModbusError::CommunicationError(format!("Failed to update device status: {}", e)))?;

        Ok(())
    }

    // Batch insert flowmeter readings (without units)
    pub async fn batch_insert_flowmeter_readings(&self, readings: Vec<FlowmeterReading>) -> Result<usize, ModbusError> {
        if readings.is_empty() {
            return Ok(0);
        }

        let batch_size = self.config.batch_size;
        let mut total_inserted = 0;

        for chunk in readings.chunks(batch_size) {
            let inserted = self.insert_flowmeter_chunk(chunk).await?;
            total_inserted += inserted;
        }

        info!("💾 Inserted {} flowmeter readings", total_inserted);
        Ok(total_inserted)
    }

    // Insert flowmeter chunk with transaction (using Unix timestamps)
    async fn insert_flowmeter_chunk(&self, readings: &[FlowmeterReading]) -> Result<usize, ModbusError> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to start transaction: {}", e))
        })?;

        let mut inserted_count = 0;

        for reading in readings {
            let result = sqlx::query(r#"
                INSERT INTO flowmeter_readings (
                    device_uuid, device_address, device_name, device_location, unix_timestamp,
                    mass_flow_rate, density_flow, temperature, volume_flow_rate,
                    mass_total, volume_total, mass_inventory, volume_inventory,
                    error_code, ipc_uuid, site_id, batch_id, quality_flag, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#)
            .bind(&reading.device_uuid)
            .bind(reading.device_address)
            .bind(&reading.device_name)
            .bind(&reading.device_location)
            .bind(reading.unix_timestamp)
            .bind(reading.mass_flow_rate)
            .bind(reading.density_flow)
            .bind(reading.temperature)
            .bind(reading.volume_flow_rate)
            .bind(reading.mass_total)
            .bind(reading.volume_total)
            .bind(reading.mass_inventory)
            .bind(reading.volume_inventory)
            .bind(reading.error_code)
            .bind(&reading.ipc_uuid)
            .bind(&reading.site_id)
            .bind(&reading.batch_id)
            .bind(&reading.quality_flag)
            .bind(reading.created_at)
            .execute(&mut *tx)
            .await;

            match result {
                Ok(_) => inserted_count += 1,
                Err(e) => {
                    warn!("Failed to insert flowmeter reading: {}", e);
                }
            }
        }

        tx.commit().await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to commit transaction: {}", e))
        })?;

        Ok(inserted_count)
    }

    // Get recent flowmeter readings
    pub async fn get_recent_flowmeter_readings(&self, limit: i64, offset: i64) -> Result<Vec<FlowmeterReading>, ModbusError> {
        let readings = sqlx::query_as::<_, FlowmeterReading>(r#"
            SELECT * FROM flowmeter_readings 
            ORDER BY unix_timestamp DESC 
            LIMIT ? OFFSET ?
        "#)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| ModbusError::CommunicationError(format!("Failed to fetch flowmeter readings: {}", e)))?;

        Ok(readings)
    }

    // Get flowmeter readings by device
    pub async fn get_device_flowmeter_readings(
        &self, 
        device_uuid: &str, 
        start_time: Option<DateTime<Utc>>, 
        end_time: Option<DateTime<Utc>>,
        limit: Option<i64>
    ) -> Result<Vec<FlowmeterReading>, ModbusError> {
        let mut query = "SELECT * FROM flowmeter_readings WHERE device_uuid = ?".to_string();
        let mut bindings = vec![device_uuid.to_string()];

        if let Some(start) = start_time {
            query.push_str(" AND unix_timestamp >= ?");
            bindings.push(start.timestamp().to_string());
        }

        if let Some(end) = end_time {
            query.push_str(" AND unix_timestamp <= ?");
            bindings.push(end.timestamp().to_string());
        }

        query.push_str(" ORDER BY unix_timestamp DESC");

        if let Some(limit_val) = limit {
            query.push_str(" LIMIT ?");
            bindings.push(limit_val.to_string());
        }

        let mut query_builder = sqlx::query_as::<_, FlowmeterReading>(&query);
        for binding in bindings {
            query_builder = query_builder.bind(binding);
        }

        let readings = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ModbusError::CommunicationError(format!("Failed to fetch device flowmeter readings: {}", e)))?;

        Ok(readings)
    }

    // Get flowmeter statistics
    pub async fn get_flowmeter_stats(&self) -> Result<FlowmeterStats, ModbusError> {
        let stats = sqlx::query_as::<_, FlowmeterStats>(r#"
            SELECT 
                COUNT(*) as total_readings,
                COUNT(DISTINCT device_uuid) as active_devices,
                AVG(mass_flow_rate) as avg_mass_flow_rate,
                MAX(mass_flow_rate) as max_mass_flow_rate,
                MIN(mass_flow_rate) as min_mass_flow_rate,
                AVG(temperature) as avg_temperature,
                MAX(unix_timestamp) as latest_reading,
                MIN(unix_timestamp) as earliest_reading
            FROM flowmeter_readings
        "#)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| ModbusError::CommunicationError(format!("Failed to fetch flowmeter stats: {}", e)))?;

        Ok(stats)
    }

    // Database maintenance operations
    pub async fn cleanup_old_data(&self, days_to_keep: i64) -> Result<u64, ModbusError> {
        let cutoff_timestamp = (Utc::now() - chrono::Duration::days(days_to_keep)).timestamp();
        
        let result = sqlx::query("DELETE FROM device_readings WHERE unix_timestamp < ?")
            .bind(cutoff_timestamp)
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

        let one_hour_ago = (Utc::now() - chrono::Duration::hours(1)).timestamp();
        let active_devices: i64 = sqlx::query_scalar(r#"
            SELECT COUNT(DISTINCT device_uuid) FROM device_readings 
            WHERE unix_timestamp > ?
        "#)
        .bind(one_hour_ago)
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

// KEEP DatabaseStats here but export it
#[derive(Debug)]
pub struct DatabaseStats {
    pub total_readings: i64,
    pub active_devices: i64,
    pub database_size_bytes: u64,
    pub connection_count: usize,
}