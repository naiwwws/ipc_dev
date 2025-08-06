use serialport::SerialPort;
use std::io::{Read, Write};
use std::time::Duration;
use anyhow::{Result};
use nmea::Nmea;
use chrono::{NaiveTime, Utc};
use log::{debug, info, warn};

use super::gps_data::{GpsData, parse_coordinate};
use crate::utils::error::ModbusError;

pub struct GpsReader {
    port: Box<dyn SerialPort>,
    nmea_parser: Nmea,
}

impl GpsReader {
    pub fn new(port_name: &str, baud_rate: u32) -> Result<Self, ModbusError> {
        info!("🧭 Initializing GPS on port {} at {} baud", port_name, baud_rate);
        
        let port = serialport::new(port_name, baud_rate)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| ModbusError::ConnectionError(format!("Failed to open GPS port {}: {}", port_name, e)))?;

        let nmea_parser = Nmea::default();

        Ok(Self {
            port,
            nmea_parser,
        })
    }

    pub fn configure_quectel_gps(&mut self) -> Result<(), ModbusError> {
        info!("🔧 Configuring Quectel GPS module...");
        
        // Configure GPS NMEA type
        self.send_at_command("AT+QGPSCFG=\"gpsnmeatype\",1")?;
        std::thread::sleep(Duration::from_millis(500));
        
        self.send_at_command("AT+QGPS?")?;
        std::thread::sleep(Duration::from_millis(500));
        
        self.send_at_command("AT+CFUN=1")?;
        std::thread::sleep(Duration::from_millis(500));
        
        self.send_at_command("AT+QGPSEND")?;
        std::thread::sleep(Duration::from_millis(500));

        // Turn on GPS
        self.send_at_command("AT+QGPS=1")?;
        std::thread::sleep(Duration::from_millis(1000));
        
        self.send_at_command("AT+QGPSLOC=0")?;
        std::thread::sleep(Duration::from_millis(1000));
        
        info!("✅ GPS module configured successfully");
        Ok(())
    }

    fn send_at_command(&mut self, command: &str) -> Result<String, ModbusError> {
        debug!("📤 Sending GPS command: {}", command);
        
        // Send command
        self.port.write_all(format!("{}\r\n", command).as_bytes())
            .map_err(|e| ModbusError::CommunicationError(format!("Failed to send GPS command: {}", e)))?;
        self.port.flush()
            .map_err(|e| ModbusError::CommunicationError(format!("Failed to flush GPS port: {}", e)))?;
        
        // Read response with timeout
        let mut response = String::new();
        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(5);
        
        while start_time.elapsed() < timeout {
            let mut buffer = [0u8; 256];
            match self.port.read(&mut buffer) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buffer[..n]);
                    response.push_str(&data);
                    
                    if response.contains("OK") || response.contains("ERROR") {
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    continue;
                }
                Err(e) => {
                    warn!("⚠️ Error reading GPS response: {}", e);
                    break;
                }
            }
        }
        
        // Debug response lines
        for line in response.lines() {
            let line = line.trim();
            if !line.is_empty() {
                debug!("📥 GPS Response: {}", line);
            }
        }
        
        Ok(response)
    }

    pub fn read_gps_continuous<F>(&mut self, mut callback: F) -> Result<(), ModbusError>
    where
        F: FnMut(GpsData) -> bool,
    {
        let mut line_buffer = String::new();
        let mut last_location_request = std::time::Instant::now();
        
        info!("📡 Starting continuous GPS monitoring...");

        loop {
            // Request location every 5 seconds
            if last_location_request.elapsed() > Duration::from_secs(5) {
                if let Err(e) = self.port.write_all(b"AT+QGPSLOC=0\r\n") {
                    warn!("⚠️ Failed to send GPS location request: {}", e);
                } else {
                    self.port.flush().ok();
                }
                last_location_request = std::time::Instant::now();
            }
            
            // Read data from port
            let mut buffer = [0u8; 1024];
            match self.port.read(&mut buffer) {
                Ok(0) => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buffer[..n]);
                    line_buffer.push_str(&data);
                    
                    // Process complete lines
                    while let Some(newline_pos) = line_buffer.find('\n') {
                        let line_owned = line_buffer[..newline_pos].trim_end_matches('\r').trim().to_string();
                        line_buffer.drain(..=newline_pos);
                        
                        if line_owned.is_empty() {
                            continue;
                        }
                        
                        debug!("📥 Received GPS data: {}", line_owned);
                        
                        // Handle QGPSLOC response
                        if line_owned.starts_with("+QGPSLOC:") {
                            if let Some(gps_data) = Self::parse_qgpsloc_static(&line_owned) {
                                debug!("✅ Parsed QGPSLOC data: {:?}", gps_data);
                                if !callback(gps_data) {
                                    return Ok(());
                                }
                            }
                        }
                        // Handle standard NMEA sentences
                        else if line_owned.starts_with('$') {
                            match self.nmea_parser.parse(&line_owned) {
                                Ok(sentence_type) => {
                                    debug!("✅ Parsed NMEA sentence: {:?}", sentence_type);
                                    
                                    // Start with current Unix timestamp
                                    let mut gps_data = GpsData::new(); // Uses current Unix timestamp
                                    
                                    if let Some(lat) = self.nmea_parser.latitude {
                                        gps_data.latitude = Some(lat);
                                    }
                                    if let Some(lon) = self.nmea_parser.longitude {
                                        gps_data.longitude = Some(lon);
                                    }
                                    if let Some(alt) = self.nmea_parser.altitude {
                                        gps_data.altitude = Some(alt);
                                    }
                                    if let Some(speed) = self.nmea_parser.speed_over_ground {
                                        gps_data.speed = Some(speed);
                                    }
                                    if let Some(course) = self.nmea_parser.true_course {
                                        gps_data.course = Some(course);
                                    }
                                    if let Some(sats) = self.nmea_parser.num_of_fix_satellites {
                                        gps_data.satellites = Some(sats);
                                    }
                                    if let Some(fix_type) = self.nmea_parser.fix_type {
                                        gps_data.fix_type = Some(format!("{:?}", fix_type));
                                    }
                                    
                                    // Convert NMEA time to Unix timestamp if available
                                    if let Some(time) = self.nmea_parser.fix_time {
                                        if let Some(date) = self.nmea_parser.fix_date {
                                            let datetime = date.and_time(time);
                                            gps_data.timestamp = Some(datetime.and_utc().timestamp());
                                        }
                                    }
                                    // If no NMEA time, keep the current timestamp set in new()

                                    if gps_data.latitude.is_some() && gps_data.longitude.is_some() {
                                        debug!("📍 GPS Fix acquired: lat={:.6}, lon={:.6}, timestamp={}", 
                                               gps_data.latitude.unwrap(), 
                                               gps_data.longitude.unwrap(),
                                               gps_data.timestamp.unwrap_or(0));
                                        if !callback(gps_data) {
                                            return Ok(());
                                        }
                                    }
                                }
                                Err(e) => {
                                    debug!("⚠️ Failed to parse NMEA sentence: {} - Error: {:?}", line_owned, e);
                                }
                            }
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    continue;
                }
                Err(e) => {
                    warn!("⚠️ GPS read error: {}", e);
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    fn parse_qgpsloc_static(line: &str) -> Option<GpsData> {
        if !line.starts_with("+QGPSLOC:") {
            return None;
        }
        
        let data_part = line.strip_prefix("+QGPSLOC: ")?;
        let parts: Vec<&str> = data_part.split(',').collect();
        
        if parts.len() < 11 {
            warn!("⚠️ Invalid QGPSLOC format: {}", line);
            return None;
        }
        
        // Start with current Unix timestamp
        let mut gps_data = GpsData::new();
        
        // Parse time and convert to Unix timestamp
        if let Ok(time_str) = parts[0].parse::<f64>() {
            let hours = (time_str / 10000.0) as u32;
            let minutes = ((time_str % 10000.0) / 100.0) as u32;
            let seconds = time_str % 100.0;
            
            if let Some(time) = NaiveTime::from_hms_opt(hours, minutes, seconds as u32) {
                // Use current date with GPS time for more accurate timestamp
                let today = Utc::now().date_naive();
                if let Some(datetime) = today.and_time(time).and_local_timezone(Utc).single() {
                    gps_data.timestamp = Some(datetime.timestamp());
                    debug!("🕐 GPS time parsed: {}", datetime.format("%Y-%m-%d %H:%M:%S UTC"));
                }
            }
        }
        
        // Parse latitude
        if let Some(lat_str) = parts.get(1) {
            if let Some(lat) = parse_coordinate(lat_str) {
                gps_data.latitude = Some(lat);
            }
        }
        
        // Parse longitude
        if let Some(lon_str) = parts.get(2) {
            if let Some(lon) = parse_coordinate(lon_str) {
                gps_data.longitude = Some(lon);
            }
        }
        
        // Parse altitude
        if let Ok(altitude) = parts[4].parse::<f32>() {
            gps_data.altitude = Some(altitude);
        }
        
        // Parse fix type
        if let Ok(fix) = parts[5].parse::<u32>() {
            gps_data.fix_type = Some(match fix {
                0 => "No fix".to_string(),
                1 => "Dead reckoning".to_string(),
                2 => "2D fix".to_string(),
                3 => "3D fix".to_string(),
                _ => format!("Unknown({})", fix),
            });
        }
        
        // Parse course
        if let Ok(course) = parts[6].parse::<f32>() {
            gps_data.course = Some(course);
        }
        
        // Parse speed (convert from km/h to knots)
        if let Ok(speed_kmh) = parts[7].parse::<f32>() {
            gps_data.speed = Some(speed_kmh * 0.539957);
        }
        
        // Parse satellites
        if let Ok(sats) = parts[10].parse::<u32>() {
            gps_data.satellites = Some(sats);
        }
        
        debug!("🧭 Parsed QGPSLOC: lat={:?}, lon={:?}, timestamp={}",
               gps_data.latitude, gps_data.longitude, gps_data.timestamp.unwrap_or(0));
        
        Some(gps_data)
    }
}