use thiserror::Error;

#[derive(Error, Debug)]
pub enum ModbusError {
    #[error("Connection error: {0}")]
    ConnectionError(String),
    
    #[error("Communication error: {0}")]
    CommunicationError(String),
    
    #[error("CRC checksum mismatch")]
    CrcError,
    
    #[error("Invalid response from device")]
    InvalidResponse,
    
    #[error("Invalid data: {0}")]
    InvalidData(String),
    
    #[error("Device not found: address {0}")]
    InvalidDevice(u8),
    
    #[error("Lock acquisition failed")]
    LockError,
    
    #[error("Timeout occurred")]
    Timeout,

    #[error("Device not found: {0}")]
    DeviceNotFound(String),
}