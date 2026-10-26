/// Process configuration read from the environment.
#[derive(Debug, Clone)]
pub struct Config {
    pub addr: String,
    pub database_url: String,
    /// Enables `POST /api/v1/auth/sessions`, which issues a session for any email.
    /// Development only; keep it off in any deployed environment.
    pub dev_login: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set".to_string())?;
        let dev_login = std::env::var("GATEWAY_DEV_LOGIN")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        Ok(Self {
            addr: std::env::var("GATEWAY_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
            database_url,
            dev_login,
        })
    }
}
