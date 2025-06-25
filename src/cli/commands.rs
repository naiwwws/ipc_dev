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

    Ok(false)
}