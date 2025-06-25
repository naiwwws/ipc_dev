pub mod sqlite_manager;
pub mod models;

pub use models::{FlowmeterReading, FlowmeterStats, DeviceStatus, SystemMetrics};
pub use sqlite_manager::{SqliteManager, DatabaseStats}; // ✅ Add DatabaseStats export