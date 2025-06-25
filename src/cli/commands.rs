use clap::ArgMatches;
use log::info;

use crate::services::DataService;
use crate::devices::flowmeter::FlowmeterDevice;
use crate::output::{JsonFormatter, CsvFormatter, FileSender, NetworkSender, DatabaseSender, MqttSender};
use crate::output::raw_sender::{RawDataSender, RawDataFormat};

pub async fn handle_subcommands(
    matches: &ArgMatches,
    service: &mut DataService,
) -> Result<bool, Box<dyn std::error::Error>> {
    
    // Configure output format
    if let Some(format) = matches.get_one::<String>("format") {
        match format.as_str() {
            "json" => {
                info!("🎨 Using JSON formatter");
                service.set_formatter(Box::new(JsonFormatter));
            }
            "csv" => {
                info!("🎨 Using CSV formatter");
                service.set_formatter(Box::new(CsvFormatter));
            }
            _ => {} // Keep default console formatter
        }
    }

    // Configure output destinations
    if let Some(output_file) = matches.get_one::<String>("output-file") {
        info!("📝 Adding file output: {}", output_file);
        service.add_sender(Box::new(FileSender::new(output_file, true)));
    }

    // Handle subcommands
    if let Some(_matches) = matches.subcommand_matches("getdata") {
        info!("🔍 Executing getdata command...");
        
        service.read_all_devices_once().await?;
        service.print_all_device_data().await?;  // This should trigger file output
        
        return Ok(true);
    }

    if let Some(matches) = matches.subcommand_matches("getvolatile") {
        info!("📈 Executing getvolatile command...");
        
        service.read_all_devices_once().await?;
        
        let parameter = matches.get_one::<String>("parameter").unwrap();
        let value = service.get_volatile_data(parameter);
        
        if value.is_empty() {
            println!("❌ No data found for parameter: {}", parameter);
            println!("💡 Available parameters: MassFlowRate, Temperature, DensityFlow, VolumeFlowRate, MassTotal, VolumeTotal, MassInventory, VolumeInventory, ErrorCode");
        } else {
            println!("📈 {}: {}", parameter, value);
        }
        
        return Ok(true);
    }

    if let Some(matches) = matches.subcommand_matches("resetaccumulation") {
        let device_addr: u8 = matches.get_one::<String>("device_address").unwrap().parse()?;
        
        info!("🔄 Executing reset accumulation for device {}...", device_addr);
        service.reset_accumulation(device_addr).await?;
        println!("✅ Reset accumulation command sent to device {}", device_addr);
        
        return Ok(true);
    }

    // Handle getrawdata command
    if let Some(matches) = matches.subcommand_matches("getrawdata") {
        info!("🔍 Executing getrawdata command...");
        
        let device_address: u8 = matches.get_one::<String>("device").unwrap().parse()
            .map_err(|_| "Invalid device address")?;
        
        let format = matches.get_one::<String>("format").unwrap_or(&"hex".to_string());
        
        // if let Some(output_file) = matches.get_one::<String>("output") {
        //     service.get_raw_data_with_file(device_address, format, output_file).await?; // Use correct method name
        // } else {
        //     service.get_raw_device_data(device_address, format).await?; // Use correct method name
        // }
        
        return Ok(true);
    }

    // Handle compare-raw command
    if let Some(matches) = matches.subcommand_matches("compare-raw") {
        info!("🔍 Executing compare-raw command...");
        
        let device_address: u8 = matches.get_one::<String>("device").unwrap().parse()
            .map_err(|_| "Invalid device address")?;
        
        service.compare_raw_vs_processed(device_address).await?; // Use correct method name
        
        return Ok(true);
    }

    if let Some(db_matches) = matches.subcommand_matches("db") {
        return handle_database_commands(db_matches).await;
    }

    Ok(false)
}

// Add this new function
async fn handle_database_commands(matches: &ArgMatches) -> Result<bool, Box<dyn std::error::Error>> {
    match matches.subcommand() {
        Some(("init", _)) => {
            println!("🗄️  Initializing database...");
            std::process::Command::new("mkdir")
                .args(["-p", "data"])
                .output()?;
                
            let init_script = r#"
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
    timestamp TEXT NOT NULL,
    batch_id TEXT,
    ipc_uuid TEXT NOT NULL,
    site_id TEXT NOT NULL,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_device_timestamp ON device_readings(device_uuid, timestamp);
CREATE INDEX IF NOT EXISTS idx_device_address ON device_readings(device_address);
CREATE INDEX IF NOT EXISTS idx_timestamp ON device_readings(timestamp);
CREATE INDEX IF NOT EXISTS idx_parameter ON device_readings(parameter_name);
"#;
            
            std::process::Command::new("sqlite3")
                .args(["data/sensor_data.db", init_script])
                .output()?;
                
            println!("✅ Database initialized successfully");
            Ok(true)
        }
        
        Some(("stats", _)) => {
            println!("📊 Database Statistics:");
            let output = std::process::Command::new("sqlite3")
                .args(["data/sensor_data.db", 
                      "SELECT 'Total readings: ' || COUNT(*) FROM device_readings; SELECT 'Unique devices: ' || COUNT(DISTINCT device_address) FROM device_readings; SELECT 'Database size: ' || page_count || ' pages' FROM pragma_page_count();"])
                .output()?;
            println!("{}", String::from_utf8_lossy(&output.stdout));
            Ok(true)
        }
        
        Some(("query", sub_matches)) => {
            let table = sub_matches.get_one::<String>("table").unwrap();
            let limit = sub_matches.get_one::<String>("limit").unwrap();
            
            let query = if let Some(device) = sub_matches.get_one::<String>("device") {
                format!("SELECT device_address, parameter_name, parameter_value, parameter_unit, timestamp FROM {} WHERE device_address = {} ORDER BY timestamp DESC LIMIT {};", 
                       table, device, limit)
            } else {
                format!("SELECT device_address, parameter_name, parameter_value, parameter_unit, timestamp FROM {} ORDER BY timestamp DESC LIMIT {};", 
                       table, limit)
            };
            
            println!("📋 Query Results:");
            let output = std::process::Command::new("sqlite3")
                .args(["-header", "-column", "data/sensor_data.db", &query])
                .output()?;
            println!("{}", String::from_utf8_lossy(&output.stdout));
            Ok(true)
        }
        
        Some(("schema", _)) => {
            println!("🏗️  Database Schema:");
            let output = std::process::Command::new("sqlite3")
                .args(["data/sensor_data.db", ".schema"])
                .output()?;
            println!("{}", String::from_utf8_lossy(&output.stdout));
            Ok(true)
        }
        
        _ => {
            println!("❌ Unknown database command");
            Ok(true)
        }
    }
}