pub mod flowmeter;
pub mod traits;
pub mod gps; // NEW: GPS module

pub use traits::{Device, DeviceData};
pub use flowmeter::{FlowmeterDevice, FlowmeterData};
 // NEW: GPS exports