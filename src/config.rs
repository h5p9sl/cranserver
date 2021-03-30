use log::warn;
use serde::Deserialize;

#[derive(Deserialize)]
/// A struct that holds the server's configuration values
pub struct Config {
    pub client_port: u16,
    pub control_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            client_port: 8080,
            control_port: 8090,
        }
    }
}

/// Looks for the configuration file in default locations
fn find_cfg() -> Option<std::path::PathBuf> {
    let locations = vec!["cranserver.toml", "/etc/cranserver/cranserver.toml"];

    for l in &locations {
        let p = std::path::PathBuf::from(l);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Attempts to find and parse config file, or returns default config if no config exists.
pub fn get_config() -> Result<Config, Box<dyn std::error::Error>> {
    if let Some(cfg) = find_cfg() {
        Ok(toml::from_str(&std::fs::read_to_string(cfg)?)?)
    } else {
        warn!("No config found. Using default settings");
        Ok(Default::default())
    }
}
