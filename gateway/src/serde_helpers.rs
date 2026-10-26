//! Serde helpers for PATCH bodies.

use serde::{Deserialize, Deserializer};

/// Distinguishes "field absent" (outer `None`, via `#[serde(default)]`) from
/// "field set to null" (`Some(None)`) and "field set to a value" (`Some(Some(v))`).
pub fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
