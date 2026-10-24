//! Reader for BlinderKitten `.olga` show files.

pub mod attribute;
pub mod light;
pub mod mirror;
pub mod model;
pub mod organic;
pub mod placement;
pub mod plans;
pub mod semantics;
pub mod short_name;
#[cfg(feature = "watch")]
pub mod watch;

use std::path::Path;

pub use attribute::Attribute;
pub use model::*;
pub use semantics::{AttrValue, FixtureClass, FixtureState};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("{path} is not a valid .olga file: {source}")]
    Json { path: String, source: serde_json::Error },
}

pub fn load(path: impl AsRef<Path>) -> Result<Show, Error> {
    let path = path.as_ref();
    let display = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|source| Error::Io { path: display.clone(), source })?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|source| Error::Json { path: display, source })?;
    Ok(Show::from_json(&json))
}
