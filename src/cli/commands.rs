use clap::ArgMatches;
use log::info;
use anyhow::{Result, anyhow}; //  Add anyhow macro import

use crate::services::DataService;
use crate::output::{JsonFormatter, CsvFormatter, FileSender, MqttSender};
use crate::output::raw_sender::{RawDataSender, RawDataFormat};

pub async fn handle_subcommands(
    matches: &ArgMatches,
    service: &mut DataService,
) -> Result<bool> {  
    
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
        service.print_all_device_data().await?;
        
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
        println!(" Reset accumulation command sent to device {}", device_addr);
        
        return Ok(true);
    }

    // Handle getrawdata command
    if let Some(matches) = matches.subcommand_matches("getrawdata") {
        info!("🔍 Executing getrawdata command...");
        
        let device_address: u8 = matches.get_one::<String>("device").unwrap().parse()
            .map_err(|_| anyhow!("Invalid device address"))?; //  Fix this line
        
        let default_format = "hex".to_string();
        let format = matches.get_one::<String>("format").unwrap_or(&default_format);
        
        if let Some(output_file) = matches.get_one::<String>("output") {
            service.read_raw_device_data(device_address, format, Some(output_file)).await?;
        } else {
            service.read_raw_device_data(device_address, format, None).await?;
        }
        
        return Ok(true);
    }

    // Handle compare-raw command
    if let Some(matches) = matches.subcommand_matches("compare-raw") {
        info!("🔍 Executing compare-raw command...");
        
        let device_address: u8 = matches.get_one::<String>("device").unwrap().parse()
            .map_err(|_| anyhow!("Invalid device address"))?; //  Fix this line
        
        service.compare_raw_vs_processed(device_address).await?;
        
        return Ok(true);
    }

    // Handle database commands (modify existing db handling)
    if let Some(matches) = matches.subcommand_matches("db") {
        if let Some(sub_matches) = matches.subcommand_matches("query") {
            let device_address: Option<u8> = sub_matches.get_one::<String>("device")
                .and_then(|s| s.parse().ok());
            let limit: i64 = sub_matches.get_one::<String>("limit")
                .unwrap_or(&"10".to_string())
                .parse()
                .map_err(|_| anyhow!("Invalid limit"))?; //  Fix this line
                
            if let Some(addr) = device_address {
                service.query_flowmeter_data(addr, limit).await?;
            } else {
                // Query all devices
                println!("📋 Querying all devices (last {}):", limit);
                // Add logic for querying all devices
            }
            return Ok(true);
        }
        
        if let Some(_) = matches.subcommand_matches("stats") {
            service.get_flowmeter_stats().await?;
            return Ok(true);
        }
        
        // Handle other db subcommands...
    }

    //  ADD: Handle flowmeter commands
    if let Some(matches) = matches.subcommand_matches("flowmeter") {
        if let Some(sub_matches) = matches.subcommand_matches("query") {
            info!("📋 Executing flowmeter query command...");
            
            let device_address: u8 = sub_matches.get_one::<String>("device").unwrap().parse()
                .map_err(|_| anyhow!("Invalid device address"))?;
            let limit: i64 = sub_matches.get_one::<String>("limit").unwrap_or(&"10".to_string()).parse()
                .map_err(|_| anyhow!("Invalid limit"))?;
                
            service.query_flowmeter_data(device_address, limit).await?;
            return Ok(true);
        }
        
        if let Some(_) = matches.subcommand_matches("stats") {
            info!("📊 Executing flowmeter stats command...");
            service.get_flowmeter_stats().await?;
            return Ok(true);
        }
        
        if let Some(sub_matches) = matches.subcommand_matches("recent") {
            info!("📋 Executing flowmeter recent command...");
            
            let limit: i64 = sub_matches.get_one::<String>("limit").unwrap_or(&"20".to_string()).parse()
                .map_err(|_| anyhow!("Invalid limit"))?;
                
            if let Some(db_service) = service.get_database_service() {
                let readings = db_service.get_recent_flowmeter_readings(limit).await?;
                
                println!("📋 Recent flowmeter readings (last {}):", limit);
                println!("{:<12} {:<12} {:<12} {:<12} {:<8} {:<25}", 
                    "Mass Flow", "Temperature", "Density", "Vol Flow", "Error", "Timestamp");
                println!("{}", "-".repeat(80));
                
                for reading in readings {
                    println!("{:<12} {:<12} {:<12} {:<12} {:<8} {:<25}", 
                        format!("{:.2}", reading.mass_flow_rate),
                        format!("{:.1}", reading.temperature),
                        format!("{:.4}", reading.density_flow),
                        format!("{:.3}", reading.volume_flow_rate),
                        reading.error_code,
                        reading.timestamp.format("%H:%M:%S")
                    );
                }
            } else {
                println!("❌ Database service not available");
            }
            return Ok(true);
        }
    }

    Ok(false)
}