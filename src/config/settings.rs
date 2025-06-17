use clap::ArgMatches;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub serial_port: String,
    pub baud_rate: u32,
    pub device_addresses: Vec<u8>,
    pub update_interval_seconds: u64,
    pub timeout_ms: u64,
    pub parity: ParityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParityConfig {
    None,
    Even,
    Odd,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            serial_port: "/dev/ttyS0".to_string(),
            baud_rate: 9600,
            device_addresses: vec![2, 3],
            update_interval_seconds: 10,
            timeout_ms: 1000,
            parity: ParityConfig::None,
        }
    }
}

impl Config {
    pub fn from_matches(matches: &ArgMatches) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            serial_port: matches.get_one::<String>("port").unwrap().clone(),
            baud_rate: matches.get_one::<String>("baud").unwrap().parse()?,
            device_addresses: matches
                .get_one::<String>("devices")
                .unwrap()
                .split(',')
                .map(|s| s.trim().parse::<u8>())
                .collect::<Result<Vec<_>, _>>()?,
            update_interval_seconds: matches.get_one::<String>("interval").unwrap().parse()?,
            timeout_ms: 1000,
            parity: ParityConfig::None,
        })
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}