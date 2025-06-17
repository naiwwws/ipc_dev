mod services;
mod cli;
mod config;
mod modbus;
mod devices;
mod utils;
mod output;

use anyhow::Result;
use clap::{Arg, Command};
use log::info;

use services::DataService;
use config::Config;
use ipc_dev_rust::{VERSION};
use cli::commands::handle_subcommands;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let matches = Command::new("Industrial Modbus Service")
        .version(VERSION)
        .about("Modular Industrial Device Communication Service")
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .value_name("PORT")
                .help("Serial port path")
                .default_value("/dev/ttyS0"),
        )
        .arg(
            Arg::new("baud")
                .short('b')
                .long("baud")
                .value_name("BAUD")
                .help("Baud rate")
                .default_value("9600"),
        )
        .arg(
            Arg::new("devices")
                .short('d')
                .long("devices")
                .value_name("DEVICES")
                .help("Device addresses (comma-separated)")
                .default_value("2,3"),
        )
        .arg(
            Arg::new("interval")
                .short('i')
                .long("interval")
                .value_name("SECONDS")
                .help("Update interval in seconds")
                .default_value("10"),
        )
        .arg(
            Arg::new("debug")
                .long("debug")
                .short('D')
                .help("Enable debug mode with automatic data printing")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .help("Output format: console, json, csv")
                .value_parser(["console", "json", "csv"])
                .default_value("console"),
        )
        .arg(
            Arg::new("output-file")
                .long("output-file")
                .value_name("FILE")
                .help("Write output to file"),
        )
        .arg(
            Arg::new("output-http")
                .long("output-http")
                .value_name("URL")
                .help("Send output to HTTP endpoint"),
        )
        .arg(
            Arg::new("output-db")
                .long("output-db")
                .value_name("CONNECTION")
                .help("Send output to database"),
        )
        .arg(
            Arg::new("output-mqtt")
                .long("output-mqtt")
                .value_name("BROKER,TOPIC")
                .help("Send output to MQTT broker (format: broker_url,topic)"),
        )
        .subcommand(
            Command::new("getdata")
                .about("Get all device data")
        )
        .subcommand(
            Command::new("getvolatile")
                .about("Get volatile data for specific parameter")
                .arg(
                    Arg::new("parameter")
                        .help("Parameter name")
                        .required(true)
                        .index(1),
                )
        )
        .subcommand(
            Command::new("resetaccumulation")
                .about("Reset accumulation for a device")
                .arg(
                    Arg::new("device_address")
                        .help("Device address to reset")
                        .required(true)
                        .index(1),
                ),
        )
        .get_matches();

    let config = Config::from_matches(&matches)?;
    let mut service = DataService::new(config.clone()).await?;

    // Handle subcommands
    if handle_subcommands(&matches, &mut service).await? {
        return Ok(());
    }

    // Check if debug mode is enabled
    let debug_mode = matches.get_flag("debug");

    // Configure output format and destinations for debug mode
    if debug_mode {
        if let Some(format) = matches.get_one::<String>("format") {
            match format.as_str() {
                "json" => {
                    info!("🎨 Using JSON formatter");
                    service.set_formatter(Box::new(crate::output::JsonFormatter));
                }
                "csv" => {
                    info!("🎨 Using CSV formatter");
                    service.set_formatter(Box::new(crate::output::CsvFormatter));
                }
                _ => {} // Keep default console formatter
            }
        }

        if let Some(output_file) = matches.get_one::<String>("output-file") {
            info!("📝 Adding file output: {}", output_file);
            service.add_sender(Box::new(crate::output::FileSender::new(output_file, true)));
        }
    }

    // Start continuous service
    info!("🚀 Starting Industrial Modbus Service version {}", VERSION);
    info!("📡 Serial port: {}", config.serial_port);
    info!("⚙️  Baud rate: {}", config.baud_rate);
    info!("🎯 Device addresses: {:?}", config.device_addresses);
    info!("⏱️  Update interval: {} seconds", config.update_interval_seconds);
    
    if debug_mode {
        info!("🐛 Debug mode enabled - automatic data printing");
    } else {
        info!("ℹ️  Use --debug flag for automatic data printing");
    }

    service.run(debug_mode).await?;

    Ok(())
}
