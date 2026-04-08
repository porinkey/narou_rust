use anyhow::Result;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_DOWNLOAD_INTERVAL: f64 = 0.7;
const DEFAULT_UPDATE_INTERVAL: f64 = 0.0;
const DEFAULT_WAIT_STEPS: u64 = 10;
const DEFAULT_RETRY_LIMIT: usize = 5;
const DEFAULT_RETRY_WAIT_SECONDS: f64 = 10.0;
const DEFAULT_LONG_WAIT_SECONDS: f64 = 5.0;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub download_interval_secs: f64,
    pub update_interval_secs: f64,
    pub download_wait_steps: u64,
    pub retry_limit: usize,
    pub retry_wait_secs: f64,
    pub long_wait_secs: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            download_interval_secs: DEFAULT_DOWNLOAD_INTERVAL,
            update_interval_secs: DEFAULT_UPDATE_INTERVAL,
            download_wait_steps: DEFAULT_WAIT_STEPS,
            retry_limit: DEFAULT_RETRY_LIMIT,
            retry_wait_secs: DEFAULT_RETRY_WAIT_SECONDS,
            long_wait_secs: DEFAULT_LONG_WAIT_SECONDS,
        }
    }
}

impl AppConfig {
    pub fn load(workspace_root: &Path) -> Result<Self> {
        let mut config = Self::default();
        let local_settings = load_setting_map(local_setting_path(workspace_root))?;
        let global_settings = load_setting_map(global_setting_path())?;

        if let Some(value) = get_f64(&local_settings, "download.interval") {
            config.download_interval_secs = value.max(0.0);
        }
        if let Some(value) = get_f64(&local_settings, "update.interval") {
            config.update_interval_secs = value.max(0.0);
        }
        if let Some(value) = get_u64(&local_settings, "download.wait-steps") {
            config.download_wait_steps = normalize_wait_steps(value);
        }

        if let Some(value) = get_f64(&global_settings, "download.retry-wait-seconds") {
            config.retry_wait_secs = value.max(0.0);
        }
        if let Some(value) = get_u64(&global_settings, "download.retry-limit") {
            config.retry_limit = value as usize;
        }
        if let Some(value) = get_f64(&global_settings, "download.long-wait-seconds") {
            config.long_wait_secs = value.max(config.download_interval_secs);
        }

        Ok(config)
    }
}

pub fn local_setting_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".narou").join("local_setting.yaml")
}

fn normalize_wait_steps(value: u64) -> u64 {
    match value {
        0 => DEFAULT_WAIT_STEPS,
        1..=10 => value,
        _ => DEFAULT_WAIT_STEPS,
    }
}

pub fn global_setting_path() -> PathBuf {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".narousetting").join("global_setting.yaml")
}

fn load_setting_map(path: PathBuf) -> Result<BTreeMap<String, serde_yaml::Value>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(path)?;
    let map = serde_yaml::from_str::<BTreeMap<String, serde_yaml::Value>>(&text)?;
    Ok(map)
}

fn get_f64(map: &BTreeMap<String, serde_yaml::Value>, key: &str) -> Option<f64> {
    let value = map.get(key)?;
    match value {
        serde_yaml::Value::Number(number) => number.as_f64(),
        serde_yaml::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn get_u64(map: &BTreeMap<String, serde_yaml::Value>, key: &str) -> Option<u64> {
    let value = map.get(key)?;
    match value {
        serde_yaml::Value::Number(number) => number.as_u64(),
        serde_yaml::Value::String(text) => text.parse::<u64>().ok(),
        _ => None,
    }
}

#[derive(Debug, Default, Deserialize)]
struct _CompatibilityMarker {}
