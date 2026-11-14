use std::env;
use std::ffi::OsString;
use std::fmt::Display;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use furigana4epub_core::{default_dictionary_path, default_wordlists_directory};

pub struct Config {
    pub bind_addr: SocketAddr,
    pub data_dir: PathBuf,
    pub dictionary_path: PathBuf,
    pub wordlists_dir: PathBuf,
    pub workers: usize,
    pub max_input_bytes: u64,
    pub max_wordlist_bytes: u64,
    pub max_storage_bytes: u64,
    pub upload_ttl: Duration,
    pub finished_ttl: Duration,
    pub cleanup_interval: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            bind_addr: value("API_BIND_ADDR", "127.0.0.1:8080")?,
            data_dir: env::var_os("API_DATA_DIR")
                .unwrap_or_else(|| OsString::from(".furigana4epub-api"))
                .into(),
            dictionary_path: path("API_DICTIONARY_PATH", default_dictionary_path)?,
            wordlists_dir: path("API_WORDLISTS_DIR", default_wordlists_directory)?,
            workers: positive("API_MAX_CONCURRENT_JOBS", "2")?.try_into()?,
            max_input_bytes: positive("API_MAX_INPUT_BYTES", "104857600")?,
            max_wordlist_bytes: positive("API_MAX_WORDLIST_BYTES", "1048576")?,
            max_storage_bytes: positive("API_MAX_STORAGE_BYTES", "2147483648")?,
            upload_ttl: duration("API_UPLOAD_TTL_SECONDS", "1800")?,
            finished_ttl: duration("API_FINISHED_TTL_SECONDS", "86400")?,
            cleanup_interval: duration("API_CLEANUP_INTERVAL_SECONDS", "60")?,
        })
    }
}

fn path(name: &str, default: fn() -> Result<PathBuf>) -> Result<PathBuf> {
    env::var_os(name).map_or_else(default, |path| Ok(path.into()))
}

fn value<T: FromStr>(name: &str, default: &str) -> Result<T>
where
    T::Err: Display,
{
    let raw = match env::var(name) {
        Ok(raw) => raw,
        Err(env::VarError::NotPresent) => default.to_owned(),
        Err(error) => return Err(error).with_context(|| format!("reading {name}")),
    };
    raw.parse()
        .map_err(|error| anyhow!("invalid {name}: {error}"))
}

fn positive(name: &str, default: &str) -> Result<u64> {
    let number: i64 = value(name, default)?;
    ensure!(number > 0, "{name} must be positive");
    Ok(number as u64)
}

fn duration(name: &str, default: &str) -> Result<Duration> {
    let seconds = positive(name, default)?;
    ensure!(seconds <= i64::MAX as u64 / 1000, "{name} is too large");
    Ok(Duration::from_secs(seconds))
}
