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
use config::{Config, DynamicConfigManager};
use ipc_dev_rust::{VERSION};
use cli::commands::handle_subcommands;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let matches = Command::new("Industrial Modbus Service")
        .version(VERSION)
        .about("Modular Industrial Device Communication Service")
        .arg(
            Arg::new("config-file")
                .long("config-file")
                .short('c')
                .value_name("FILE")
                .help("Configuration file path")
                .default_value("setup/default.toml"),
        )
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
                .help("Output format: console, json, csv, hex")
                .value_parser(["console", "json", "csv", "hex"])
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
        .subcommand(
            Command::new("getrawdata")
                .about("Get raw device data for debugging")
                .arg(
                    Arg::new("device")
                        .long("device")
                        .short('d')
                        .value_name("ADDRESS")
                        .help("Specific device address (optional)")
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .short('f')
                        .value_name("FORMAT")
                        .help("Output format: debug, hex, json, binary")
                        .value_parser(["debug", "hex", "json", "binary"])
                        .default_value("debug")
                )
                .arg(
                    Arg::new("output")
                        .long("output")
                        .short('o')
                        .value_name("FILE")
                        .help("Output file path")
                )
        )
        .subcommand(
            Command::new("compare-raw")
                .about("Compare raw vs processed data")
                .arg(
                    Arg::new("device")
                        .help("Device address")
                        .required(true)
                        .index(1)
                )
        )
        .subcommand(
            Command::new("config")
                .about("Dynamic configuration management")
                .subcommand(
                    Command::new("show")
                        .about("Show current configuration")
                )
                .subcommand(
                    Command::new("ipc")
                        .about("IPC configuration management")
                        .subcommand(
                            Command::new("set-name")
                                .about("Set IPC name")
                                .arg(Arg::new("name").help("IPC name").required(true).index(1))
                                .arg(Arg::new("operator").long("operator").help("Operator name").default_value("CLI"))
                        )
                        .subcommand(
                            Command::new("regenerate-uuid")
                                .about("Generate new UUID for this IPC")
                        )
                )
                .subcommand(
                    Command::new("set-interval")
                        .about("Set polling interval")
                        .arg(
                            Arg::new("seconds")
                                .help("Polling interval in seconds")
                                .required(true)
                                .index(1)
                                .value_parser(clap::value_parser!(u64).range(1..=3600))
                        )
                        .arg(
                            Arg::new("operator")
                                .long("operator")
                                .help("Operator name")
                                .default_value("CLI")
                        )
                )
                .subcommand(
                    Command::new("set")
                        .about("Set configuration parameter")
                        .arg(Arg::new("target").help("Target (serial, device:ADDRESS, monitoring, site)").required(true).index(1))
                        .arg(Arg::new("key").help("Parameter key").required(true).index(2))
                        .arg(Arg::new("value").help("Parameter value").required(true).index(3))
                        .arg(Arg::new("operator").long("operator").help("Operator name").default_value("CLI"))
                )
                .subcommand(
                    Command::new("add")
                        .about("Add new device")
                        .arg(Arg::new("address").help("Device address").required(true).index(1))
                        .arg(Arg::new("device-id").long("device-id").help("Device ID"))
                        .arg(Arg::new("device-type").long("device-type").help("Device type").default_value("flowmeter"))
                        .arg(Arg::new("name").long("name").help("Device name"))
                        .arg(Arg::new("location").long("location").help("Device location"))
                        .arg(Arg::new("operator").long("operator").help("Operator name").default_value("CLI"))
                )
                .subcommand(
                    Command::new("enable")
                        .about("Enable device")
                        .arg(Arg::new("address").help("Device address").required(true).index(1))
                        .arg(Arg::new("operator").long("operator").help("Operator name").default_value("CLI"))
                )
                .subcommand(
                    Command::new("disable")
                        .about("Disable device")
                        .arg(Arg::new("address").help("Device address").required(true).index(1))
                        .arg(Arg::new("operator").long("operator").help("Operator name").default_value("CLI"))
                )
                .subcommand(
                    Command::new("remove")
                        .about("Remove device")
                        .arg(Arg::new("address").help("Device address").required(true).index(1))
                        .arg(Arg::new("operator").long("operator").help("Operator name").default_value("CLI"))
                )
                .subcommand(
                    Command::new("backup")
                        .about("Backup configuration")
                        .arg(Arg::new("name").long("name").help("Backup name"))
                        .arg(Arg::new("operator").long("operator").help("Operator name").default_value("CLI"))
                )
                .subcommand(
                    Command::new("restore")
                        .about("Restore configuration")
                        .arg(Arg::new("name").help("Backup name").required(true).index(1))
                        .arg(Arg::new("operator").long("operator").help("Operator name").default_value("CLI"))
                )
                .subcommand(
                    Command::new("reset")
                        .about("Reset configuration to defaults")
                )
        )
        .get_matches();

    // Handle config commands FIRST, before creating the service
    if let Some(config_matches) = matches.subcommand_matches("config") {
        let config_file = matches.get_one::<String>("config-file").unwrap();
        let backup_dir = "setup/backups";
        
        // Create config directory if it doesn't exist
        std::fs::create_dir_all("config").unwrap_or_else(|e| {
            eprintln!("Warning: Could not create config directory: {}", e);
        });
        std::fs::create_dir_all(backup_dir).unwrap_or_else(|e| {
            eprintln!("Warning: Could not create backup directory: {}", e);
        });
        
        // Create config manager
        let config_manager = DynamicConfigManager::new(config_file, backup_dir)?;
        
        // Handle config commands
        let handled = crate::config::config_commands::handle_config_commands(config_matches, &config_manager).await?;
        
        if handled {
            return Ok(());
        }
    }

    // Load config from file or command line
    let config_file = matches.get_one::<String>("config-file").unwrap();
    let config = if std::path::Path::new(config_file).exists() {
        info!("📁 Loading configuration from: {}", config_file);
        Config::from_file(config_file).unwrap_or_else(|e| {
            eprintln!("⚠️  Failed to load config file: {}, using CLI args", e);
            Config::from_matches(&matches).unwrap_or_else(|e| {
                eprintln!("❌ Failed to parse CLI args: {}, using defaults", e);
                Config::default()
            })
        })
    } else {
        info!("📁 Config file not found, using CLI args and defaults");
        let config = Config::from_matches(&matches).unwrap_or_else(|e| {
            eprintln!("❌ Failed to parse CLI args: {}, using defaults", e);
            Config::default()
        });
        
        // Save the config file for future use
        if let Err(e) = config.save_to_file(config_file) {
            eprintln!("⚠️  Failed to save initial config file: {}", e);
        } else {
            info!("💾 Created initial config file: {}", config_file);
        }
        
        config
    };

    let mut service = DataService::new(config.clone()).await?;

    // Handle other subcommands
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
                "hex" => {
                    info!("🔍 Using Hex formatter");
                    service.set_formatter(Box::new(crate::output::HexFormatter));
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
