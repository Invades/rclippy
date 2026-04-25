use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::APP_NAME;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub listen_addr: String,
    pub peer_addr: String,
    pub start_on_login: bool,
    pub poll_ms: u64,
    pub max_text_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:38765".to_owned(),
            peer_addr: String::new(),
            start_on_login: false,
            poll_ms: 500,
            max_text_bytes: 1_048_576,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        self.listen_socket_addr()
            .context("listen_addr must be a valid ip:port")?;

        if !self.peer_addr.trim().is_empty() {
            self.peer_socket_addr()
                .context("peer_addr must be empty or a valid ip:port")?;
        }

        if self.poll_ms < 100 {
            bail!("poll_ms must be at least 100");
        }

        if self.max_text_bytes == 0 {
            bail!("max_text_bytes must be greater than zero");
        }

        Ok(())
    }

    pub fn listen_socket_addr(&self) -> Result<SocketAddr> {
        Ok(self.listen_addr.parse()?)
    }

    pub fn peer_socket_addr(&self) -> Result<SocketAddr> {
        Ok(self.peer_addr.parse()?)
    }

    pub fn poll_interval(&self) -> Duration {
        Duration::from_millis(self.poll_ms)
    }

    pub fn has_peer_addr(&self) -> bool {
        !self.peer_addr.trim().is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", APP_NAME).context("no platform config directory")?;
        Ok(Self {
            path: dirs.config_dir().join("config.toml"),
        })
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Config> {
        if !self.path.exists() {
            let config = Config::default();
            self.save(&config)?;
            return Ok(config);
        }

        let text = fs::read_to_string(&self.path)
            .with_context(|| format!("read config {}", self.path.display()))?;
        let config: Config = toml::from_str(&text).context("parse config TOML")?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        config.validate()?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create config dir {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(config).context("serialize config")?;
        fs::write(&self.path, text).with_context(|| format!("write config {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn rejects_bad_peer_addr() {
        let config = Config {
            peer_addr: "nope".to_owned(),
            ..Config::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn round_trips_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::at(dir.path().join("config.toml"));
        let config = Config {
            peer_addr: "127.0.0.1:38766".to_owned(),
            start_on_login: true,
            ..Config::default()
        };

        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
    }
}
