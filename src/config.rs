//! User configuration.
//!
//! Loaded from `~/.config/devdoctor/config.toml`. If the file is missing the
//! defaults below are used. If the file is present but malformed the load
//! returns an error so the user sees a useful message.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default command timeout in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 5;

/// Default list of tools to detect.
pub const DEFAULT_TOOLS: &[&str] = &[
    "git",
    "docker",
    "go",
    "rustc",
    "cargo",
    "node",
    "npm",
    "pnpm",
    "python3",
    "php",
    "composer",
    "kubectl",
    "helm",
    "terraform",
    "aws",
];

/// Default list of TCP ports to probe.
pub const DEFAULT_PORTS: &[u16] = &[3000, 3306, 5432, 6379, 8000, 8080, 9000];

/// Top-level configuration struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Default command timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,

    /// Tools configuration.
    #[serde(default)]
    pub tools: ToolsConfig,

    /// Ports configuration.
    #[serde(default)]
    pub ports: PortsConfig,

    /// Kubernetes configuration.
    #[serde(default)]
    pub kubernetes: KubernetesConfig,
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            tools: ToolsConfig::default(),
            ports: PortsConfig::default(),
            kubernetes: KubernetesConfig::default(),
        }
    }
}

impl Config {
    /// Return the configured timeout as a [`Duration`].
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_seconds)
    }

    /// Resolve the list of tools to check, applying defaults if absent.
    pub fn resolved_tools(&self) -> Vec<String> {
        match &self.tools.enabled {
            Some(list) => list.clone(),
            None => DEFAULT_TOOLS.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// Resolve the list of ports to probe, applying defaults if absent.
    pub fn resolved_ports(&self) -> Vec<u16> {
        match &self.ports.check {
            Some(list) => list.clone(),
            None => DEFAULT_PORTS.to_vec(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// If `Some`, restricts which tools are checked. If `None`, defaults apply.
    pub enabled: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortsConfig {
    /// If `Some`, restricts which ports are probed. If `None`, defaults apply.
    pub check: Option<Vec<u16>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubernetesConfig {
    /// Emit a warning when the current namespace is `default`.
    #[serde(default = "default_true")]
    pub warn_default_namespace: bool,
}

fn default_true() -> bool {
    true
}

impl Default for KubernetesConfig {
    fn default() -> Self {
        Self {
            warn_default_namespace: true,
        }
    }
}

/// Errors produced while loading configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read config file at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse config file at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

/// Default config path: `$XDG_CONFIG_HOME/devdoctor/config.toml` or
/// `~/.config/devdoctor/config.toml`.
pub fn default_config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("devdoctor").join("config.toml"));
        }
    }
    if let Some(home) = dirs_home() {
        return Some(home.join(".config").join("devdoctor").join("config.toml"));
    }
    None
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Load the configuration from `path`.
///
/// If `path` does not exist, returns [`Config::default`]. I/O and parse
/// failures surface as [`ConfigError`].
pub fn load_from(path: &Path) -> Result<Config, ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(source) => Err(ConfigError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Load the configuration from the default location, falling back to defaults
/// if no file exists.
pub fn load_default() -> Result<Config, ConfigError> {
    match default_config_path() {
        Some(p) => load_from(&p),
        None => Ok(Config::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_config_is_sensible() {
        let c = Config::default();
        assert_eq!(c.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
        assert!(c.kubernetes.warn_default_namespace);
        assert!(c.resolved_tools().contains(&"git".to_string()));
        assert!(c.resolved_ports().contains(&3000));
    }

    #[test]
    fn empty_file_uses_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).expect("create");
        writeln!(f).expect("write");
        drop(f);

        let c = load_from(&path).expect("load");
        assert_eq!(c.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
    }

    #[test]
    fn custom_values_are_parsed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
timeout_seconds = 12

[tools]
enabled = ["git", "go"]

[ports]
check = [1234, 5678]

[kubernetes]
warn_default_namespace = false
"#,
        )
        .expect("write");

        let c = load_from(&path).expect("load");
        assert_eq!(c.timeout_seconds, 12);
        assert_eq!(c.resolved_tools(), vec!["git", "go"]);
        assert_eq!(c.resolved_ports(), vec![1234, 5678]);
        assert!(!c.kubernetes.warn_default_namespace);
    }

    #[test]
    fn malformed_config_produces_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is not = valid = toml ==").expect("write");
        assert!(load_from(&path).is_err());
    }

    #[test]
    fn missing_config_returns_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.toml");
        let c = load_from(&path).expect("load");
        assert_eq!(c.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
    }
}
