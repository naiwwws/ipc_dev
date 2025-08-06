use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use log::{info, warn, error};

use super::{GpsData, GpsReader};
use crate::utils::error::ModbusError;

#[derive(Clone)] // Make GpsService cloneable
pub struct GpsService {
    current_data: Arc<RwLock<GpsData>>,
    is_running: Arc<RwLock<bool>>,
    port_name: String,
    baud_rate: u32,
}

impl GpsService {
    pub fn new(port_name: String, baud_rate: u32) -> Self {
        Self {
            current_data: Arc::new(RwLock::new(GpsData::default())),
            is_running: Arc::new(RwLock::new(false)),
            port_name,
            baud_rate,
        }
    }
    
    pub async fn start(&self) -> Result<(), ModbusError> {
        let mut running = self.is_running.write().await;
        if *running {
            info!("🧭 GPS service already running");
            return Ok(());
        }
        
        *running = true;
        drop(running);
        
        let port_name = self.port_name.clone();
        let baud_rate = self.baud_rate;
        let current_data = self.current_data.clone();
        let is_running = self.is_running.clone();
        
        tokio::spawn(async move {
            info!("🧭 Starting GPS service on port {} at {} baud", port_name, baud_rate);
            
            while *is_running.read().await {
                match GpsReader::new(&port_name, baud_rate) {
                    Ok(mut gps_reader) => {
                        info!("🧭 GPS reader initialized successfully");
                        
                        // Configure GPS module
                        if let Err(e) = gps_reader.configure_quectel_gps() {
                            error!("❌ Failed to configure GPS module: {}", e);
                            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                            continue;
                        }
                        
                        let current_data_clone = current_data.clone();
                        let is_running_clone = is_running.clone();
                        
                        let gps_task = tokio::task::spawn_blocking(move || {
                            let result = gps_reader.read_gps_continuous(move |gps_data| {
                                // Use try_lock to avoid blocking
                                if let Ok(mut data) = current_data_clone.try_write() {
                                    *data = gps_data.clone();
                                    
                                    // Log GPS updates
                                    if gps_data.has_valid_fix() {
                                        info!("📍 GPS Fix: {:.6}°, {:.6}° | {} sats", 
                                              gps_data.latitude.unwrap_or(0.0),
                                              gps_data.longitude.unwrap_or(0.0),
                                              gps_data.satellites.unwrap_or(0));
                                    }
                                }
                                
                                // Check if we should continue (non-blocking)
                                if let Ok(running) = is_running_clone.try_read() {
                                    *running
                                } else {
                                    true // Continue if we can't check
                                }
                            });
                            
                            if let Err(e) = result {
                                error!("❌ GPS monitoring error: {}", e);
                            }
                        });
                        
                        // Wait for the GPS task or stop signal
                        tokio::select! {
                            _ = gps_task => {
                                info!("🧭 GPS monitoring task completed");
                            }
                            _ = tokio::time::sleep(tokio::time::Duration::from_secs(60)) => {
                                info!("🧭 GPS monitoring cycle timeout, restarting...");
                            }
                        }
                    },
                    Err(e) => {
                        error!("❌ Failed to initialize GPS reader: {}", e);
                    }
                }
                
                // Wait before retrying
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
            
            info!("🧭 GPS service stopped");
        });
        
        Ok(())
    }
    
    pub async fn stop(&self) -> Result<(), ModbusError> {
        let mut running = self.is_running.write().await;
        *running = false;
        info!("🧭 GPS service stopping...");
        Ok(())
    }
    
    pub async fn get_current_data(&self) -> GpsData {
        self.current_data.read().await.clone()
    }
    
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }
    
    pub async fn get_status(&self) -> String {
        let running = self.is_running().await;
        let data = self.get_current_data().await;
        
        if running {
            if data.has_valid_fix() {
                format!("🧭 GPS Active - Position: {:.6}°, {:.6}°, Satellites: {}", 
                        data.latitude.unwrap_or(0.0), 
                        data.longitude.unwrap_or(0.0),
                        data.satellites.unwrap_or(0))
            } else {
                "🧭 GPS Active - Waiting for fix...".to_string()
            }
        } else {
            "🧭 GPS Inactive".to_string()
        }
    }
}