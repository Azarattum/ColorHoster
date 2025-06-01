use anyhow::Result;
use clap::{Parser, ValueEnum};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

use crate::consts::OPENRGB_SDK_DEFAULT_PORT;

/// Color Hoster is OpenRGB compatible high-performance SDK server for VIA per-key RGB
#[derive(Parser, Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
#[command(
    version,
    after_help = format!("{} ./ColorHoster -b -j ./p1_he_ansi_v1.0.json", "Example:".bold())
)]
pub struct CLI {
    /// Set a directory to look for VIA `.json` definitions for keyboards [default: <executable directory>]
    #[arg(short, long)]
    #[serde(skip_serializing_if = "default")]
    pub directory: Option<PathBuf>,

    /// Add a direct path to a VIA `.json` file (can be multiple)
    #[arg(short, long)]
    #[serde(skip_serializing_if = "default")]
    pub json: Vec<std::path::PathBuf>,

    /// Allow direct mode to change brightness values
    #[arg(short, long)]
    #[serde(skip_serializing_if = "default")]
    pub brightness: bool,

    /// Set a directory for storing and loading profiles [default: ./profiles]
    #[arg(long)]
    #[serde(skip_serializing_if = "default")]
    pub profiles: Option<PathBuf>,

    /// Set the port to listen on
    #[serde(default = "default_port", skip_serializing_if = "is_default_port")]
    #[arg(short, long, default_value_t = default_port())]
    pub port: u32,

    /// Manage Color Hoster service
    #[serde(skip)]
    #[arg(short, long)]
    pub service: Option<ServiceAction>,
}

#[derive(Clone, Debug, ValueEnum, Serialize, Deserialize)]
pub enum ServiceAction {
    Create,
    Delete,
    Start,
    Stop,
}

impl CLI {
    pub fn parse_args(args: impl IntoIterator<Item = String>) -> Self {
        let config = CLI::from_config().unwrap_or_default();
        let cli = CLI::parse_from(args);
        CLI {
            directory: cli.directory.or(config.directory),
            json: if cli.json.is_empty() {
                config.json
            } else {
                cli.json
            },
            brightness: cli.brightness || config.brightness,
            profiles: cli.profiles.or(config.profiles),
            port: if cli.port == 6742 {
                config.port
            } else {
                cli.port
            },
            service: cli.service.or(config.service),
        }
    }

    pub fn from_config() -> Option<CLI> {
        let path = CLI::config_path();
        if let Ok(content) = fs::read_to_string(path) {
            toml::from_str(&content).ok()
        } else {
            None
        }
    }

    pub fn save_to_config(&self) -> Result<bool> {
        let config_str = toml::to_string_pretty(self)?;
        if !config_str.trim().is_empty() {
            std::fs::write(CLI::config_path(), config_str)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn config_path() -> PathBuf {
        CLI::current_dir().join("colorhoster.toml")
    }

    pub fn current_dir() -> PathBuf {
        std::env::current_exe()
            .expect("Failed to get current executable path!")
            .parent()
            .expect("Failed to get parent directory of current executable!")
            .to_path_buf()
    }
}

impl Default for CLI {
    fn default() -> Self {
        CLI {
            directory: None,
            json: Vec::new(),
            brightness: false,
            profiles: None,
            port: OPENRGB_SDK_DEFAULT_PORT,
            service: None,
        }
    }
}

fn default_port() -> u32 {
    OPENRGB_SDK_DEFAULT_PORT
}

fn default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}

fn is_default_port(port: &u32) -> bool {
    *port == OPENRGB_SDK_DEFAULT_PORT
}
