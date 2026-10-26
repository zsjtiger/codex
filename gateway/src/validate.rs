use crate::error::ApiError;

pub const DEFAULT_PAGE_SIZE: i64 = 50;
pub const MAX_PAGE_SIZE: i64 = 100;

pub fn length(field: &str, value: &str, min: usize, max: usize) -> Result<(), ApiError> {
    let n = value.chars().count();
    if (min..=max).contains(&n) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "{field} must be between {min} and {max} characters"
        )))
    }
}

/// Normalizes an email to lowercase and checks its basic shape.
pub fn email(value: &str) -> Result<String, ApiError> {
    let normalized = value.trim().to_lowercase();
    let valid = normalized.len() <= 320
        && normalized.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        });
    if valid {
        Ok(normalized)
    } else {
        Err(ApiError::BadRequest("email is not valid".to_string()))
    }
}

/// Lowercase letters, digits and inner hyphens, 3 to 40 characters.
pub fn slug(value: &str) -> Result<(), ApiError> {
    let valid = (3..=40).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-');
    if valid {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "slug must be 3-40 lowercase letters, digits or hyphens, and cannot start or end with a hyphen"
                .to_string(),
        ))
    }
}

pub fn sha256(value: &str) -> Result<(), ApiError> {
    let valid = value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if valid {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "sha256 must be 64 lowercase hex characters".to_string(),
        ))
    }
}

pub fn endpoint(value: &str) -> Result<(), ApiError> {
    if value.starts_with("ws://") || value.starts_with("wss://") {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "endpoint must start with ws:// or wss://".to_string(),
        ))
    }
}

/// Returns `(limit, offset)` with defaults applied and bounds checked.
pub fn page(limit: Option<i64>, offset: Option<i64>) -> Result<(i64, i64), ApiError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE);
    let offset = offset.unwrap_or(0);
    if (1..=MAX_PAGE_SIZE).contains(&limit) && offset >= 0 {
        Ok((limit, offset))
    } else {
        Err(ApiError::BadRequest(format!(
            "limit must be 1-{MAX_PAGE_SIZE} and offset must be >= 0"
        )))
    }
}
