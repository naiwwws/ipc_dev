pub mod traits;
pub mod flowmeter;
pub mod registry;

pub use traits::{Device, DeviceData};
pub use flowmeter::{FlowmeterDevice, FlowmeterData};
// pub use registry::DeviceRegistry;  // ← Comment out until implemented