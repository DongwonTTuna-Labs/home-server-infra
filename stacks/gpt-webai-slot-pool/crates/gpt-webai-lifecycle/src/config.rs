use std::env;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::contracts::ids::validate_host_id;

static TEST_CLOCK_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    pub state_root: PathBuf,
    pub slot_count: u8,
    pub slot_container_prefix: String,
    pub slot_mode: String,
    pub status_provider_check: bool,
    pub provider_status_timeout_ms: u64,
}

#[derive(Debug, Error)]
pub enum HostIdError {
    #[error("host id seed path has no parent")]
    MissingParent,
    #[error("host id seed io error: {0}")]
    Io(#[from] std::io::Error),
}

impl SupervisorConfig {
    pub fn from_env() -> Self {
        let state_root = resolve_state_root(
            env::var_os("GPT_WEBAI_STATE_ROOT").as_deref(),
            env::var_os("XDG_STATE_HOME").as_deref(),
            env::var_os("HOME").as_deref(),
        )
        .expect("GPT_WEBAI_STATE_ROOT or HOME is required");
        let slot_count = env::var("GPT_WEBAI_SLOT_COUNT")
            .ok()
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(10);
        let slot_container_prefix = env::var("GPT_WEBAI_SLOT_CONTAINER_PREFIX")
            .unwrap_or_else(|_| "gpt-webai-".to_string());
        let slot_mode = env::var("GPT_WEBAI_SLOT_MODE").unwrap_or_else(|_| "auto".to_string());
        let status_provider_check = env_bool("GPT_WEBAI_RUST_STATUS_PROVIDER_CHECK", true);
        let provider_status_timeout_ms = env::var("GPT_WEBAI_PROVIDER_STATUS_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(15_000);
        Self {
            state_root,
            slot_count,
            slot_container_prefix,
            slot_mode,
            status_provider_check,
            provider_status_timeout_ms,
        }
    }

    pub fn slot_pool_enabled(&self) -> bool {
        match self.slot_mode.as_str() {
            "0" | "off" | "false" => false,
            "1" | "on" | "docker" | "fake" => true,
            _ => true,
        }
    }
}

pub fn resolve_state_root(
    configured: Option<&OsStr>,
    xdg_state_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    nonempty_path(configured).or_else(|| {
        state_base(xdg_state_home, home).map(|base| base.join("gpt-webai-lifecycle").join("r13"))
    })
}

pub fn resolve_host_id_seed_path(
    xdg_state_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    state_base(xdg_state_home, home).map(|base| base.join("gpt-webai-lifecycle/host-id"))
}

pub fn load_or_create_host_id(path: &Path) -> Result<String, HostIdError> {
    let parent = path.parent().ok_or(HostIdError::MissingParent)?;
    fs::create_dir_all(parent)?;
    let directory = File::open(parent)?;
    lock_exclusive(&directory)?;

    if let Ok(host_id) = read_host_id(path) {
        return Ok(host_id);
    }
    match fs::remove_file(path) {
        Ok(()) => directory.sync_all()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let mut entropy = [0_u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut entropy)?;
    let seed = entropy
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(seed.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    directory.sync_all()?;
    Ok(format!("host_{seed}"))
}

fn read_host_id(path: &Path) -> Result<String, HostIdError> {
    let bytes = fs::read(path)?;
    if bytes.len() != 33 || bytes.last() != Some(&b'\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid host id seed length",
        )
        .into());
    }
    let seed = std::str::from_utf8(&bytes[..32]).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "host id seed is not UTF-8")
    })?;
    let host_id = format!("host_{seed}");
    validate_host_id(&host_id).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid host id seed")
    })?;
    Ok(host_id)
}

fn lock_exclusive(file: &File) -> std::io::Result<()> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn state_base(xdg_state_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    nonempty_path(xdg_state_home)
        .or_else(|| nonempty_path(home).map(|path| path.join(".local/state")))
}

fn nonempty_path(value: Option<&OsStr>) -> Option<PathBuf> {
    value
        .filter(|item| !item.is_empty())
        .map(Path::new)
        .map(Path::to_path_buf)
}

fn env_bool(name: &str, fallback: bool) -> bool {
    match env::var(name).ok().as_deref() {
        Some("0" | "off" | "false" | "no") => false,
        Some("1" | "on" | "true" | "yes") => true,
        _ => fallback,
    }
}

pub fn now_ms() -> u64 {
    if env::var_os("GPT_WEBAI_STATE_ROOT").is_some() {
        if let Some(base) = env::var("GPT_WEBAI_TEST_EPOCH_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
        {
            let offset = TEST_CLOCK_COUNTER.fetch_add(1, Ordering::SeqCst);
            return base
                .checked_add(offset.saturating_mul(1_000))
                .expect("GPT_WEBAI_TEST_EPOCH_MS overflow");
        }
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock before Unix epoch")
        .as_millis() as u64
}

pub fn now_system_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(now_ms())
}
