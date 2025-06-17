use anyhow::Result;
use chrono::{DateTime, Utc};
use std::io::{self, Write, Read};
use std::time::Duration;
use tokio::time::sleep;
use serialport::SerialPort;

#[derive(Debug, Clone)]
pub struct ModbusDevice {
    pub id: u8,
    pub name: String,
    pub registers: Vec<RegisterInfo>,
}

#[derive(Debug, Clone)]
pub struct RegisterInfo {
    pub address: u16,
    pub count: u16,
    pub register_type: RegisterType,
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum RegisterType {
    Holding,
    Input,
    Coil,
    Discrete,
}

#[derive(Debug, Clone)]
pub struct FlowmeterData {
    pub device_address: u8,
    pub timestamp: DateTime<Utc>,
    pub error_code: u32,
    pub mass_flow_rate: f32,
    pub density_flow: f32,
    pub temperature: f32,
    pub volume_flow_rate: f32,
    pub mass_total: f32,
    pub volume_total: f32,
    pub mass_inventory: f32,
    pub volume_inventory: f32,
}

pub struct ModbusReader {
    // Use serialport::SerialPort instead of tokio_serial
    serial_port: Option<Box<dyn SerialPort>>,
    is_connected: bool,
    #[allow(dead_code)]
    devices: Vec<String>,
    response_buffer: Vec<u8>,
    debug_mode: bool,
    modbus_is_busy: bool,
}

impl ModbusReader {
    pub fn new() -> Self {
        Self {
            serial_port: None,
            is_connected: false,
            devices: Vec::new(),
            response_buffer: Vec::new(),
            debug_mode: true,
            modbus_is_busy: false,
        }
    }

    // CRC16 Modbus calculation - exactly matching Lua implementation
    fn crc16_modbus(&self, data: &[u8]) -> u16 {
        let poly = 0xA001;
        let mut crc = 0xFFFF;

        for byte in data {
            crc ^= *byte as u16;
            
            for _ in 0..8 {
                if crc & 0x01 != 0 {
                    crc = (crc >> 1) ^ poly;
                } else {
                    crc = crc >> 1;
                }
            }
        }

        crc
    }

    pub async fn list_serial_ports(&self) -> Result<()> {
        println!("📡 Available Serial Ports:");
        
        let ports = serialport::available_ports()?;
        if ports.is_empty() {
            println!("   ⚠️  No serial ports found");
            return Ok(());
        }
        
        for (index, port) in ports.iter().enumerate() {
            println!("   {}. {}", index + 1, port.port_name);
            match &port.port_type {
                serialport::SerialPortType::UsbPort(usb_info) => {
                    if let Some(manufacturer) = &usb_info.manufacturer {
                        println!("      📱 Manufacturer: {}", manufacturer);
                    }
                    if let Some(serial_number) = &usb_info.serial_number {
                        println!("      🔢 Serial Number: {}", serial_number);
                    }
                }
                _ => {}
            }
            println!();
        }
        
        Ok(())
    }

    pub async fn connect(&mut self, port: &str, baud_rate: u32) -> Result<()> {
        println!("🔌 Connecting to RS485 port: {}", port);
        println!("⚙️  Configuration: {} baud, even parity, 8 data bits, 1 stop bit", baud_rate);
        
        let serial = serialport::new(port, baud_rate)
            .data_bits(serialport::DataBits::Eight)
            .stop_bits(serialport::StopBits::One)
            .parity(serialport::Parity::Even)
            .timeout(Duration::from_millis(1000))
            .open()?;
            
        self.serial_port = Some(serial);
        self.is_connected = true;
        
        println!("✅ RS485 connection established successfully");
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if self.is_connected {
            self.serial_port = None;
            self.is_connected = false;
            self.response_buffer.clear();
            self.modbus_is_busy = false;
            println!("✅ RS485 connection closed gracefully");
        }
    }

    // Read Holding Registers - matching Lua implementation
    pub async fn read_holding_registers(
        &mut self, 
        address: u8, 
        start_register_addr: u16, 
        quantity: u16,
        timeout_ms: u64
    ) -> Result<Option<Vec<u8>>> {
        if !self.is_connected || self.serial_port.is_none() {
            return Err(anyhow::anyhow!("Not connected to RS485. Call connect() first."));
        }

        if self.modbus_is_busy {
            return Err(anyhow::anyhow!("Modbus is busy"));
        }

        // Validate inputs
        if address < 1 || address > 247 {
            return Err(anyhow::anyhow!("Invalid device address: {}. Must be 1-247", address));
        }

        // Create frame exactly like Lua
        let mut bytes = vec![
            address,
            0x03, // Read Holding Registers
            (start_register_addr >> 8) as u8,
            (start_register_addr & 0xFF) as u8,
            (quantity >> 8) as u8,
            (quantity & 0xFF) as u8,
        ];

        // Calculate and append CRC exactly like Lua
        let crc = self.crc16_modbus(&bytes);
        bytes.push((crc & 0xFF) as u8);      // CRC low byte
        bytes.push((crc >> 8) as u8);        // CRC high byte

        if self.debug_mode {
            println!("📤 Sending frame: [{}]", 
                bytes.iter().map(|b| format!("0x{:02x}", b)).collect::<Vec<_>>().join(", "));
        }

        self.modbus_is_busy = true;
        let read_length = (quantity * 2) + 5; // Data bytes + address + function + byte_count + 2 CRC

        // Send frame
        if let Some(ref mut port) = self.serial_port {
            port.write_all(&bytes)?;
        }

        // Read response with timeout
        let response = match tokio::time::timeout(
            Duration::from_millis(timeout_ms), 
            self.read_response(read_length as usize)
        ).await {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => {
                self.modbus_is_busy = false;
                return Err(e);
            }
            Err(_) => {
                self.modbus_is_busy = false;
                println!("⏰ Response timeout");
                return Ok(None);
            }
        };

        self.modbus_is_busy = false;

        // Process response exactly like Lua
        if response.len() < 5 {
            println!("❌ Response too short");
            return Ok(None);
        }

        // Check if response matches expected format
        if response[1] != 0x03 {
            println!("❌ Unexpected function code: 0x{:02x}", response[1]);
            return Ok(None);
        }

        // Verify CRC exactly like Lua
        let mut response_data = response.clone();
        let crc_high = response_data.pop().unwrap();
        let crc_low = response_data.pop().unwrap();
        let received_crc = self.crc16_modbus(&response_data);

        if (received_crc & 0xFF) as u8 != crc_low || (received_crc >> 8) as u8 != crc_high {
            println!("❌ CRC validation failed");
            if self.debug_mode {
                println!("   📐 Calculated CRC: 0x{:04x} (Low: 0x{:02x}, High: 0x{:02x})", 
                    received_crc, received_crc & 0xFF, received_crc >> 8);
                println!("   📨 Received CRC: Low=0x{:02x}, High=0x{:02x}", crc_low, crc_high);
            }
            return Ok(None);
        }

        // Extract data exactly like Lua - remove address, function code, and byte count
        let mut data = response_data;
        data.remove(0); // address
        data.remove(0); // function code
        data.remove(0); // byte count

        if self.debug_mode {
            println!("✅ Valid response received: [{}]", 
                data.iter().map(|b| format!("0x{:02x}", b)).collect::<Vec<_>>().join(", "));
        }

        Ok(Some(data))
    }

    // Write Single Register - matching Lua implementation
    pub async fn write_single_register(
        &mut self,
        address: u8,
        register_addr: u16,
        value: u16,
        timeout_ms: u64
    ) -> Result<bool> {
        if !self.is_connected || self.serial_port.is_none() {
            return Err(anyhow::anyhow!("Not connected to RS485. Call connect() first."));
        }

        if self.modbus_is_busy {
            return Err(anyhow::anyhow!("Modbus is busy"));
        }

        // Create frame exactly like Lua
        let mut bytes = vec![
            address,
            0x06, // Write Single Register
            (register_addr >> 8) as u8,
            (register_addr & 0xFF) as u8,
            (value >> 8) as u8,
            (value & 0xFF) as u8,
        ];

        // Calculate and append CRC
        let crc = self.crc16_modbus(&bytes);
        bytes.push((crc & 0xFF) as u8);
        bytes.push((crc >> 8) as u8);

        if self.debug_mode {
            println!("📤 Write Single Register frame: [{}]", 
                bytes.iter().map(|b| format!("0x{:02x}", b)).collect::<Vec<_>>().join(", "));
        }

        self.modbus_is_busy = true;
        let read_length = 8; // Echo response length

        // Send frame
        if let Some(ref mut port) = self.serial_port {
            port.write_all(&bytes)?;
        }

        // Read response
        let response = match tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            self.read_response(read_length)
        ).await {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => {
                self.modbus_is_busy = false;
                return Err(e);
            }
            Err(_) => {
                self.modbus_is_busy = false;
                println!("⏰ Write response timeout");
                return Ok(false);
            }
        };

        self.modbus_is_busy = false;

        // Verify response (should echo the request)
        if response.len() != 8 || response[1] != 0x06 {
            println!("❌ Invalid write response");
            return Ok(false);
        }

        // Verify CRC
        let mut response_data = response.clone();
        let crc_high = response_data.pop().unwrap();
        let crc_low = response_data.pop().unwrap();
        let received_crc = self.crc16_modbus(&response_data);

        if (received_crc & 0xFF) as u8 != crc_low || (received_crc >> 8) as u8 != crc_high {
            println!("❌ Write response CRC validation failed");
            return Ok(false);
        }

        println!("✅ Write Single Register successful");
        Ok(true)
    }

    // Write Single Coil - matching Lua implementation  
    pub async fn write_single_coil(
        &mut self,
        address: u8,
        coil_addr: u16,
        value: bool,
        timeout_ms: u64
    ) -> Result<bool> {
        if !self.is_connected || self.serial_port.is_none() {
            return Err(anyhow::anyhow!("Not connected to RS485. Call connect() first."));
        }

        if self.modbus_is_busy {
            return Err(anyhow::anyhow!("Modbus is busy"));
        }

        // Create frame exactly like Lua
        let mut bytes = vec![
            address,
            0x05, // Write Single Coil
            (coil_addr >> 8) as u8,
            (coil_addr & 0xFF) as u8,
            if value { 0xFF } else { 0x00 }, // Coil value
            0x00,
        ];

        // Calculate and append CRC
        let crc = self.crc16_modbus(&bytes);
        bytes.push((crc & 0xFF) as u8);
        bytes.push((crc >> 8) as u8);

        if self.debug_mode {
            println!("📤 Write Single Coil frame: [{}]", 
                bytes.iter().map(|b| format!("0x{:02x}", b)).collect::<Vec<_>>().join(", "));
        }

        self.modbus_is_busy = true;
        let read_length = 8;

        // Send frame
        if let Some(ref mut port) = self.serial_port {
            port.write_all(&bytes)?;
        }

        // Read response
        let response = match tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            self.read_response(read_length)
        ).await {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => {
                self.modbus_is_busy = false;
                return Err(e);
            }
            Err(_) => {
                self.modbus_is_busy = false;
                println!("⏰ Coil write response timeout");
                return Ok(false);
            }
        };

        self.modbus_is_busy = false;

        // Verify response
        if response.len() != 8 || response[1] != 0x05 {
            println!("❌ Invalid coil write response");
            return Ok(false);
        }

        // Verify CRC
        let mut response_data = response.clone();
        let crc_high = response_data.pop().unwrap();
        let crc_low = response_data.pop().unwrap();
        let received_crc = self.crc16_modbus(&response_data);

        if (received_crc & 0xFF) as u8 != crc_low || (received_crc >> 8) as u8 != crc_high {
            println!("❌ Coil write response CRC validation failed");
            return Ok(false);
        }

        println!("✅ Write Single Coil successful");
        Ok(true)
    }

    // Fixed read_response method for serialport crate
    async fn read_response(&mut self, expected_length: usize) -> Result<Vec<u8>> {
        let mut response = Vec::new();
        let mut buffer = [0u8; 256];

        // Give some time for the response to arrive
        sleep(Duration::from_millis(50)).await;

        let timeout = Duration::from_millis(2000);
        let start_time = std::time::Instant::now();

        while response.len() < expected_length && start_time.elapsed() < timeout {
            if let Some(ref mut port) = self.serial_port {
                // Use blocking read (serialport crate is blocking)
                match port.read(&mut buffer) {
                    Ok(n) if n > 0 => {
                        response.extend_from_slice(&buffer[..n]);
                        
                        if self.debug_mode {
                            println!("📥 Raw data received: [{}] ({} bytes)", 
                                buffer[..n].iter().map(|b| format!("0x{:02x}", b)).collect::<Vec<_>>().join(", "),
                                n);
                        }

                        if response.len() >= expected_length {
                            break;
                        }
                    }
                    Ok(_) => {
                        // No data available, yield control to tokio
                        tokio::task::yield_now().await;
                        sleep(Duration::from_millis(10)).await;
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                        // Timeout on individual read, continue waiting
                        if !response.is_empty() {
                            tokio::task::yield_now().await;
                        }
                        sleep(Duration::from_millis(10)).await;
                    }
                    Err(e) => {
                        return Err(anyhow::anyhow!("Read error: {}", e));
                    }
                }
            }
        }

        if response.is_empty() {
            return Err(anyhow::anyhow!("No response received"));
        }

        if self.debug_mode {
            println!("📥 Complete response: [{}] ({} bytes)", 
                response.iter().map(|b| format!("0x{:02x}", b)).collect::<Vec<_>>().join(", "),
                response.len());
        }

        Ok(response)
    }

    // Test device responsiveness
    pub async fn test_device_responsive(&mut self, device_address: u8) -> Result<bool> {
        if !self.is_connected {
            return Ok(false);
        }

        let max_attempts = 3;
        for attempt in 1..=max_attempts {
            println!("🔍 Testing device {} (attempt {}/{})...", device_address, attempt, max_attempts);

            match self.read_holding_registers(device_address, 244, 1, 2000).await {
                Ok(Some(_)) => {
                    println!("✅ Device {} is responsive", device_address);
                    return Ok(true);
                }
                Ok(None) => {
                    println!("📵 Device {} test attempt {} failed", device_address, attempt);
                }
                Err(e) => {
                    println!("❌ Device {} test error: {}", device_address, e);
                }
            }

            if attempt < max_attempts {
                sleep(Duration::from_secs(1)).await;
            }
        }

        println!("📵 Device {} is not responsive after {} attempts", device_address, max_attempts);
        Ok(false)
    }

    // Scan for devices
    pub async fn scan_for_devices(&mut self, address_range: &[u8]) -> Result<Vec<u8>> {
        println!("🔍 Scanning for devices on addresses: [{}]", 
            address_range.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "));
        
        let mut responsive_devices = Vec::new();
        
        for &address in address_range {
            println!("\n🔍 Scanning device {}...", address);
            
            match self.test_device_responsive(address).await {
                Ok(true) => {
                    responsive_devices.push(address);
                    println!("✅ Device {} added to responsive list", address);
                }
                Ok(false) => {
                    println!("❌ Device {} scan failed", address);
                }
                Err(e) => {
                    println!("❌ Device {} scan failed: {}", address, e);
                }
            }
            
            // Delay between device scans
            sleep(Duration::from_millis(500)).await;
        }
        
        println!("\n📊 Scan Summary:");
        println!("   ✅ Responsive devices: [{}]", 
            responsive_devices.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "));
        
        let non_responsive: Vec<u8> = address_range.iter()
            .filter(|&&addr| !responsive_devices.contains(&addr))
            .cloned()
            .collect();
        println!("   📵 Non-responsive: [{}]", 
            non_responsive.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "));
        
        let success_rate = (responsive_devices.len() as f32 / address_range.len() as f32) * 100.0;
        println!("   📈 Success rate: {:.1}%", success_rate);
        
        Ok(responsive_devices)
    }

    // Convert 32-bit integer to float
    fn int32_to_float(&self, value: u32) -> f32 {
        f32::from_bits(value)
    }

    // Read flowmeter data (customize based on your device)
    pub async fn read_flowmeter_data(&mut self, device_address: u8) -> Result<Option<FlowmeterData>> {
        if !self.is_connected {
            return Err(anyhow::anyhow!("Not connected to RS485. Call connect() first."));
        }

        // Read registers for flowmeter data
        let start_address = 244; // Adjust based on your device
        let register_count = 22;  // Adjust based on your data structure
        
        println!("📊 Reading flowmeter data from device {}", device_address);
        
        match self.read_holding_registers(device_address, start_address, register_count, 5000).await? {
            Some(data) => {
                if data.len() < 44 { // 22 registers * 2 bytes each
                    println!("❌ Insufficient data received");
                    return Ok(None);
                }

                // Parse flowmeter data (adjust based on your device's data format)
                let flowmeter_data = FlowmeterData {
                    device_address,
                    timestamp: Utc::now(),
                    // Parse your specific data format here
                    error_code: ((data[0] as u32) << 24) | 
                                ((data[1] as u32) << 16) |
                                ((data[2] as u32) << 8) | 
                                (data[3] as u32),
                    mass_flow_rate: self.int32_to_float(
                        ((data[4] as u32) << 24) |
                        ((data[5] as u32) << 16) |
                        ((data[6] as u32) << 8) |
                        (data[7] as u32)
                    ),
                    density_flow: self.int32_to_float(
                        ((data[8] as u32) << 24) |
                        ((data[9] as u32) << 16) |
                        ((data[10] as u32) << 8) |
                        (data[11] as u32)
                    ),
                    temperature: self.int32_to_float(
                        ((data[12] as u32) << 24) |
                        ((data[13] as u32) << 16) |
                        ((data[14] as u32) << 8) |
                        (data[15] as u32)
                    ),
                    volume_flow_rate: self.int32_to_float(
                        ((data[16] as u32) << 24) |
                        ((data[17] as u32) << 16) |
                        ((data[18] as u32) << 8) |
                        (data[19] as u32)
                    ),
                    mass_total: 0.0,      
                    volume_total: 0.0,    
                    mass_inventory: 0.0,  
                    volume_inventory: 0.0,
                };

                println!("✅ Successfully parsed flowmeter data from device {}:", device_address);
                println!("   🏷️  Error Code: 0x{:x}", flowmeter_data.error_code);
                println!("   ⚖️  Mass Flow Rate: {:.3} kg/h", flowmeter_data.mass_flow_rate);
                println!("   🌡️  Temperature: {:.2} °C", flowmeter_data.temperature);
                println!("   💧 Volume Flow Rate: {:.3} m³/h", flowmeter_data.volume_flow_rate);

                Ok(Some(flowmeter_data))
            }
            None => {
                println!("❌ No valid response from device {}", device_address);
                Ok(None)
            }
        }
    }

    // Monitor flowmeters continuously
    pub async fn monitor_flowmeters(&mut self, device_addresses: &[u8], interval: Duration) -> Result<()> {
        println!("🔄 Starting flowmeter monitoring");
        println!("   📡 Devices: [{}]", 
            device_addresses.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "));
        println!("   ⏱️  Update interval: {:?}", interval);
        println!("   🛑 Press Ctrl+C to stop\n");
        
        let mut successful_reads = 0u32;
        let mut failed_reads = 0u32;
        
        let mut interval_timer = tokio::time::interval(interval);
        
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("\n🛑 Stopping flowmeter monitor...");
                    break;
                }
                _ = interval_timer.tick() => {
                    println!("\n⏰ {} - Reading flowmeter data...", Utc::now().format("%Y-%m-%d %H:%M:%S"));
                    
                    for &device_address in device_addresses {
                        match self.read_flowmeter_data(device_address).await {
                            Ok(Some(data)) => {
                                successful_reads += 1;
                                println!("📊 Device {} - Flowmeter Data:", device_address);
                                println!("   🏷️  Error Code: 0x{:x}", data.error_code);
                                println!("   ⚖️  Mass Flow: {:.2} kg/h", data.mass_flow_rate);
                                println!("   🌡️  Temperature: {:.2} °C", data.temperature);
                                println!("   💧 Volume Flow: {:.3} m³/h", data.volume_flow_rate);
                            }
                            Ok(None) => {
                                failed_reads += 1;
                                println!("📵 Device {} - Read failed", device_address);
                            }
                            Err(e) => {
                                failed_reads += 1;
                                println!("💥 Device {} - Error: {}", device_address, e);
                            }
                        }
                    }
                    
                    // Display statistics
                    let total_reads = successful_reads + failed_reads;
                    if total_reads > 0 {
                        let success_rate = (successful_reads as f32 / total_reads as f32) * 100.0;
                        println!("📈 Success rate: {:.1}% ({}/{})", success_rate, successful_reads, total_reads);
                    }
                }
            }
        }
        
        println!("📊 Final Statistics:");
        println!("   ✅ Successful reads: {}", successful_reads);
        println!("   ❌ Failed reads: {}", failed_reads);
        
        self.disconnect();
        Ok(())
    }

    pub fn set_debug_mode(&mut self, enabled: bool) {
        self.debug_mode = enabled;
        println!("🔧 Debug mode {}", if enabled { "enabled" } else { "disabled" });
    }

    // Test CRC implementation
    pub fn test_crc_implementation(&self) {
        println!("🔧 Testing CRC implementation (Lua-compatible)...");
        
        // Test with known Modbus frame
        let test_data = [0x01, 0x03, 0x00, 0xF4, 0x00, 0x01];
        let calculated_crc = self.crc16_modbus(&test_data);
        println!("   📊 Test data: [{}]", 
            test_data.iter().map(|b| format!("0x{:02x}", b)).collect::<Vec<_>>().join(", "));
        println!("   🔢 Calculated CRC: 0x{:04x}", calculated_crc);
        println!("   📋 CRC bytes: Low=0x{:02x}, High=0x{:02x}", 
            calculated_crc & 0xFF, (calculated_crc >> 8) & 0xFF);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🖥️  Modbus Reader - Rust Version (Lua-compatible)\n");
    
    let mut reader = ModbusReader::new();
    
    // Test CRC implementation
    reader.test_crc_implementation();
    println!();
    
    // List serial ports
    reader.list_serial_ports().await?;
    println!();
    
    // Connect to RS485 (adjust port for your system)
    let port = if cfg!(target_os = "linux") {
        "/dev/ttyUSB0"
    } else if cfg!(target_os = "windows") {
        "COM1"
    } else if cfg!(target_os = "macos") {
        "/dev/tty.usbserial-0001"  // Common macOS USB serial port
    } else {
        "/dev/ttyS0"
    };
    
    match reader.connect(port, 9600).await {
        Ok(_) => {
            // Test devices
            let device_addresses = vec![1, 2, 3, 4, 5];
            let responsive_devices = reader.scan_for_devices(&device_addresses).await?;
            
            if responsive_devices.is_empty() {
                println!("❌ No responsive devices found. Check your connections and device addresses.");
                reader.disconnect();
                return Ok(());
            }
            
            // Single read example
            println!("\n📊 Single read example from responsive devices:\n");
            for address in &responsive_devices {
                match reader.read_holding_registers(*address, 244, 1, 2000).await? {
                    Some(data) => {
                        println!("✅ Device {}: Read {} bytes: [{}]", 
                            address, 
                            data.len(),
                            data.iter().map(|b| format!("0x{:02x}", b)).collect::<Vec<_>>().join(", "));
                    }
                    None => {
                        println!("❌ Device {}: No response", address);
                    }
                }
            }
            
            // Ask for continuous monitoring
            print!("\nStart continuous monitoring? (y/n): ");
            io::stdout().flush()?;
            
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            
            if input.trim().to_lowercase() == "y" || input.trim().to_lowercase() == "yes" {
                reader.monitor_flowmeters(&responsive_devices, Duration::from_secs(5)).await?;
            } else {
                // Example of other operations
                println!("\n🔧 Testing other Modbus functions...");
                
                if let Some(&first_device) = responsive_devices.first() {
                    // Test write single register
                    println!("📝 Testing Write Single Register...");
                    match reader.write_single_register(first_device, 100, 0x1234, 2000).await {
                        Ok(true) => println!("✅ Write successful"),
                        Ok(false) => println!("❌ Write failed"),
                        Err(e) => println!("💥 Write error: {}", e),
                    }
                    
                    // Test write single coil  
                    println!("📝 Testing Write Single Coil...");
                    match reader.write_single_coil(first_device, 0, true, 2000).await {
                        Ok(true) => println!("✅ Coil write successful"),
                        Ok(false) => println!("❌ Coil write failed"),
                        Err(e) => println!("💥 Coil write error: {}", e),
                    }
                }
                
                reader.disconnect();
                println!("👋 Goodbye!");
            }
        }
        Err(e) => {
            println!("❌ Failed to connect to serial port {}: {}", port, e);
            println!("💡 Try adjusting the port name in the code or check if the device is connected.");
            println!("   Common ports:");
            println!("   - Linux: /dev/ttyUSB0, /dev/ttyACM0, /dev/ttyS0");
            println!("   - Windows: COM1, COM2, COM3, etc.");
            println!("   - macOS: /dev/tty.usbserial-*, /dev/tty.usbmodem-*");
        }
    }
    
    Ok(())
}
