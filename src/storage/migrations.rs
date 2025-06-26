use log::info;
use sqlx::SqlitePool;
use crate::utils::error::ModbusError;

pub struct DatabaseMigrations;

impl DatabaseMigrations {
    pub async fn run_migrations(pool: &SqlitePool) -> Result<(), ModbusError> {
        info!("🔄 Running database migrations");
        
        // Create migration tracking table
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS migrations (
                id INTEGER PRIMARY KEY,
                version TEXT NOT NULL UNIQUE,
                applied_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
        "#).execute(pool).await.map_err(|e| {
            ModbusError::CommunicationError(format!("Failed to create migrations table: {}", e))
        })?;

        // Check and apply migrations
        Self::apply_migration_v1(pool).await?;
        Self::apply_migration_v2(pool).await?;
        
        info!(" All migrations completed");
        Ok(())
    }

    async fn apply_migration_v1(pool: &SqlitePool) -> Result<(), ModbusError> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM migrations WHERE version = 'v1')")
            .fetch_one(pool)
            .await
            .unwrap_or(false);

        if !exists {
            info!("📦 Applying migration v1: Add quality_flag column");
            
            // Add quality_flag column if it doesn't exist
            let _ = sqlx::query("ALTER TABLE device_readings ADD COLUMN quality_flag TEXT NOT NULL DEFAULT 'GOOD'")
                .execute(pool)
                .await; // Ignore error if column already exists

            // Record migration
            sqlx::query("INSERT INTO migrations (version) VALUES ('v1')")
                .execute(pool)
                .await
                .map_err(|e| ModbusError::CommunicationError(format!("Failed to record migration v1: {}", e)))?;
        }

        Ok(())
    }

    async fn apply_migration_v2(pool: &SqlitePool) -> Result<(), ModbusError> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM migrations WHERE version = 'v2')")
            .fetch_one(pool)
            .await
            .unwrap_or(false);

        if !exists {
            info!("📦 Applying migration v2: Add system_metrics table indexes");
            
            // Add additional indexes for performance
            sqlx::query("CREATE INDEX IF NOT EXISTS idx_metrics_timestamp ON system_metrics(timestamp DESC)")
                .execute(pool)
                .await
                .map_err(|e| ModbusError::CommunicationError(format!("Failed to create metrics index: {}", e)))?;

            // Record migration
            sqlx::query("INSERT INTO migrations (version) VALUES ('v2')")
                .execute(pool)
                .await
                .map_err(|e| ModbusError::CommunicationError(format!("Failed to record migration v2: {}", e)))?;
        }

        Ok(())
    }
}