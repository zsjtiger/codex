use std::{path::PathBuf, time::Duration};

use crate::runtime::{LocalLaunch, RuntimeConfig};

/// Process configuration read from the environment.
#[derive(Debug, Clone)]
pub struct Config {
    pub addr: String,
    pub database_url: String,
    /// Enables `POST /api/v1/auth/sessions`, which issues a session for any email.
    /// Development only; keep it off in any deployed environment.
    pub dev_login: bool,
    /// Path to the `codex` binary, used to start app-servers for environments with no endpoint.
    pub codex_bin: Option<PathBuf>,
    /// Root directory for per-environment `CODEX_HOME`s, required with `codex_bin`.
    pub codex_home: Option<PathBuf>,
    pub idle_timeout: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set".to_string())?;
        let dev_login = std::env::var("GATEWAY_DEV_LOGIN")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let codex_bin = std::env::var_os("GATEWAY_CODEX_BIN").map(PathBuf::from);
        let codex_home = std::env::var_os("GATEWAY_CODEX_HOME").map(PathBuf::from);
        if codex_bin.is_some() && codex_home.is_none() {
            return Err("GATEWAY_CODEX_HOME must be set when GATEWAY_CODEX_BIN is set".to_string());
        }
        let idle_secs = match std::env::var("GATEWAY_IDLE_SECS") {
            Ok(value) => value
                .parse::<u64>()
                .map_err(|_| "GATEWAY_IDLE_SECS must be a whole number of seconds".to_string())?,
            Err(_) => 30 * 60,
        };
        Ok(Self {
            addr: std::env::var("GATEWAY_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
            database_url,
            dev_login,
            codex_bin,
            codex_home,
            idle_timeout: Duration::from_secs(idle_secs),
        })
    }

    pub fn runtime_config(&self) -> RuntimeConfig {
        let local =
            self.codex_bin
                .clone()
                .zip(self.codex_home.clone())
                .map(|(codex_bin, home_root)| LocalLaunch {
                    codex_bin,
                    home_root,
                });
        RuntimeConfig {
            local,
            idle_timeout: self.idle_timeout,
        }
    }
}
