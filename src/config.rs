//! User and project configuration.
//!
//! Two optional layers are merged, later layers winning key-by-key:
//!
//! 1. the user config at `~/.config/devdoctor/config.toml`;
//! 2. a project config, `devdoctor.toml`, found in the current directory or
//!    the nearest ancestor that has one.
//!
//! Missing files are skipped and the defaults below fill any gaps. A file
//! that is present but malformed returns an error so the user sees a useful
//! message.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::version::VersionReq;

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
    /// Tools with a version requirement are always included.
    pub fn resolved_tools(&self) -> Vec<String> {
        let mut tools: Vec<String> = match &self.tools.enabled {
            Some(list) => list.clone(),
            None => DEFAULT_TOOLS.iter().map(|s| (*s).to_string()).collect(),
        };
        for tool in self.tools.versions.keys() {
            if !tools.contains(tool) {
                tools.push(tool.clone());
            }
        }
        tools
    }

    /// Resolve the list of ports to probe. Defaults apply only when no port
    /// list or expectation is configured; ports with an expectation are
    /// always included.
    pub fn resolved_ports(&self) -> Vec<u16> {
        let p = &self.ports;
        let mut ports = match &p.check {
            Some(list) => list.clone(),
            None if p.expect_listening.is_empty() && p.expect_free.is_empty() => {
                DEFAULT_PORTS.to_vec()
            }
            None => Vec::new(),
        };
        for port in p.expect_listening.iter().chain(&p.expect_free) {
            if !ports.contains(port) {
                ports.push(*port);
            }
        }
        ports
    }

    /// Cross-field checks serde can't express.
    fn validate(&self) -> Result<(), String> {
        if let Some(port) = self
            .ports
            .expect_listening
            .iter()
            .find(|p| self.ports.expect_free.contains(p))
        {
            return Err(format!(
                "port {port} is in both ports.expect_listening and ports.expect_free"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// If `Some`, restricts which tools are checked. If `None`, defaults apply.
    pub enabled: Option<Vec<String>>,

    /// Version requirements per tool, e.g. `node = ">=20"`.
    #[serde(default)]
    pub versions: BTreeMap<String, VersionReq>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortsConfig {
    /// If `Some`, restricts which ports are probed. If `None`, defaults apply.
    pub check: Option<Vec<u16>>,

    /// Ports where a service must be listening (e.g. a local database).
    #[serde(default)]
    pub expect_listening: Vec<u16>,

    /// Ports that must be free (e.g. the port the dev server binds).
    #[serde(default)]
    pub expect_free: Vec<u16>,
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
    #[error("invalid config in {path}: {message}")]
    Invalid { path: PathBuf, message: String },
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

/// File name of the per-project config.
pub const PROJECT_CONFIG_FILE: &str = "devdoctor.toml";

/// Find `devdoctor.toml` in `start` or the nearest ancestor directory.
pub fn find_project_config(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(PROJECT_CONFIG_FILE))
        .find(|p| p.is_file())
}

/// Load the configuration from `path`.
///
/// If `path` does not exist, returns [`Config::default`]. I/O and parse
/// failures surface as [`ConfigError`].
#[cfg(test)]
pub fn load_from(path: &Path) -> Result<Config, ConfigError> {
    load_layered(&[path])
}

/// Load and merge config files in order; later files override earlier ones
/// key-by-key (nested tables merge, everything else is replaced). Missing
/// files are skipped.
pub fn load_layered(paths: &[&Path]) -> Result<Config, ConfigError> {
    let mut merged = toml::Table::new();
    let mut last_path: Option<&Path> = None;
    for path in paths {
        if let Some(table) = read_table(path)? {
            merge_tables(&mut merged, table);
            last_path = Some(path);
        }
    }
    let Some(last_path) = last_path else {
        return Ok(Config::default());
    };
    let config: Config =
        toml::Value::Table(merged)
            .try_into()
            .map_err(|source| ConfigError::Parse {
                path: last_path.to_path_buf(),
                source,
            })?;
    config.validate().map_err(|message| ConfigError::Invalid {
        path: last_path.to_path_buf(),
        message,
    })?;
    Ok(config)
}

/// Read `path` as a TOML table, validating it against [`Config`] on its own
/// so errors point at the file that caused them.
fn read_table(path: &Path) -> Result<Option<toml::Table>, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    let parse_err = |source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    };
    toml::from_str::<Config>(&text).map_err(parse_err)?;
    toml::from_str::<toml::Table>(&text)
        .map(Some)
        .map_err(parse_err)
}

fn merge_tables(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge_tables(b, o),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Load the user config, then overlay the project config found from the
/// current directory. Falls back to defaults if neither exists.
pub fn load_default() -> Result<Config, ConfigError> {
    let user = default_config_path();
    let project = std::env::current_dir()
        .ok()
        .and_then(|cwd| find_project_config(&cwd));
    let paths: Vec<&Path> = user
        .iter()
        .chain(project.iter())
        .map(PathBuf::as_path)
        .collect();
    load_layered(&paths)
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
    fn project_config_overrides_user_config_per_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let user = dir.path().join("user.toml");
        let project = dir.path().join("devdoctor.toml");
        std::fs::write(
            &user,
            "timeout_seconds = 9\n[tools]\nenabled = [\"git\"]\n[ports]\ncheck = [1]\n",
        )
        .expect("write");
        std::fs::write(&project, "[tools]\nenabled = [\"go\"]\n").expect("write");

        let c = load_layered(&[&user, &project]).expect("load");
        assert_eq!(c.timeout_seconds, 9);
        assert_eq!(c.resolved_tools(), vec!["go"]);
        assert_eq!(c.resolved_ports(), vec![1]);
    }

    #[test]
    fn layered_error_names_the_bad_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let user = dir.path().join("user.toml");
        let project = dir.path().join("devdoctor.toml");
        std::fs::write(&user, "timeout_seconds = 3\n").expect("write");
        std::fs::write(&project, "timeout_seconds = \"soon\"\n").expect("write");

        let err = load_layered(&[&user, &project]).expect_err("should fail");
        assert!(err.to_string().contains("devdoctor.toml"), "{err}");
    }

    #[test]
    fn version_requirements_are_parsed_and_add_tools() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[tools]\nenabled = [\"git\"]\n[tools.versions]\nnode = \">=20\"\ngit = \">=2.40\"\n",
        )
        .expect("write");

        let c = load_from(&path).expect("load");
        assert_eq!(c.resolved_tools(), vec!["git", "node"]);
        assert_eq!(c.tools.versions["node"].to_string(), ">=20");
    }

    #[test]
    fn invalid_version_requirement_is_a_config_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tools.versions]\nnode = \"latest\"\n").expect("write");
        let err = load_from(&path).expect_err("should fail");
        assert!(
            err.to_string().contains("invalid version requirement"),
            "{err}"
        );
    }

    #[test]
    fn port_expectations_replace_defaults_and_are_probed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[ports]\nexpect_listening = [5432]\nexpect_free = [3000]\n",
        )
        .expect("write");
        let c = load_from(&path).expect("load");
        assert_eq!(c.resolved_ports(), vec![5432, 3000]);

        std::fs::write(
            &path,
            "[ports]\ncheck = [8080, 5432]\nexpect_listening = [5432]\n",
        )
        .expect("write");
        let c = load_from(&path).expect("load");
        assert_eq!(c.resolved_ports(), vec![8080, 5432]);
    }

    #[test]
    fn conflicting_port_expectations_are_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[ports]\nexpect_listening = [80]\nexpect_free = [80]\n",
        )
        .expect("write");
        let err = load_from(&path).expect_err("should fail");
        assert!(err.to_string().contains("port 80"), "{err}");
    }

    #[test]
    fn find_project_config_walks_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("mkdir");
        assert_eq!(find_project_config(&nested), None);

        let file = dir.path().join("a").join(PROJECT_CONFIG_FILE);
        std::fs::write(&file, "").expect("write");
        assert_eq!(find_project_config(&nested), Some(file));
    }

    #[test]
    fn missing_config_returns_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.toml");
        let c = load_from(&path).expect("load");
        assert_eq!(c.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
    }
}
