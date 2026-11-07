//! Password hashing for username and password sign-in.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

/// A real hash of an unguessable password. It is checked when the username does not exist, so a
/// failed sign-in takes about as long whether or not the account exists.
fn dummy_hash() -> &'static str {
    static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| {
        hash(&uuid::Uuid::new_v4().to_string()).expect("hashing a constant succeeds")
    })
}

/// Hashes a password with Argon2id and a random salt.
pub fn hash(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

/// Checks a password against a stored hash. A missing hash is checked against a dummy hash.
pub fn verify(password: &str, stored: Option<&str>) -> bool {
    let Ok(parsed) = PasswordHash::new(stored.unwrap_or(dummy_hash())) else {
        return false;
    };
    let matches = Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok();
    matches && stored.is_some()
}
