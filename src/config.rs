use serde::Deserialize;

#[derive(Deserialize)]
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

pub fn get_config() -> Result<Option<Config>, Box<dyn std::error::Error>> {
    if let Some(cfg) = find_cfg() {
        Ok(Some(toml::from_str(&std::fs::read_to_string(cfg)?)?))
    } else {
        use std::io::{Error, ErrorKind};
        Err(Box::new(Error::new(
            ErrorKind::NotFound,
            "Config file was not found.",
        )))
    }
}
