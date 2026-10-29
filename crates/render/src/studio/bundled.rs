//! The environments that come with 3DCraft (`assets/environments`, CC0
//! from Poly Haven, `ATTRIBUTION.md`), and the prepared ones, kept once
//! per process: preparing is a pure function of the picture, so the cache
//! changes no result.
//!
//! The desktop programs carry the pictures in the binary. The browser
//! build does not (they would weigh on the WebAssembly module): its page
//! fetches the one chosen and hands it over with [`register`].

use super::{Prepared, decode_hdr};
use crate::RenderError;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// The names of the bundled environments, the default first.
pub const ENVIRONMENTS: [&str; 3] = ["studio", "outdoor", "workshop"];

/// The bytes of a bundled environment's `.hdr` file, where the program
/// carries them.
pub fn bundled(name: &str) -> Option<&'static [u8]> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match name {
            "studio" => Some(include_bytes!("../../../../assets/environments/studio.hdr")),
            "outdoor" => Some(include_bytes!(
                "../../../../assets/environments/outdoor.hdr"
            )),
            "workshop" => Some(include_bytes!(
                "../../../../assets/environments/workshop.hdr"
            )),
            _ => None,
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        None
    }
}

fn cache() -> &'static Mutex<BTreeMap<String, Arc<Prepared>>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Arc<Prepared>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Prepares an environment from the bytes of its `.hdr` file and keeps it
/// under `name` (the browser's page, after fetching it).
pub fn register(name: &str, bytes: &[u8]) -> Result<Arc<Prepared>, RenderError> {
    let image = decode_hdr(bytes).map_err(|e| RenderError::Environment(e.to_string()))?;
    let prepared = Arc::new(Prepared::new(&image)?);
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(name.to_string(), Arc::clone(&prepared));
    Ok(prepared)
}

/// The prepared environment of that name, if it is ready already.
pub fn ready(name: &str) -> Option<Arc<Prepared>> {
    cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(name)
        .cloned()
}

/// The prepared environment of that name: from the cache, or prepared now
/// from the bundled picture. An error names the environments there are.
pub fn environment(name: &str) -> Result<Arc<Prepared>, RenderError> {
    if let Some(prepared) = ready(name) {
        return Ok(prepared);
    }
    match bundled(name) {
        Some(bytes) => register(name, bytes),
        None if ENVIRONMENTS.contains(&name) => Err(RenderError::Environment(format!(
            "the environment {name:?} is not loaded yet"
        ))),
        None => Err(RenderError::Environment(format!(
            "no environment {name:?}; there are {}",
            ENVIRONMENTS.join(", ")
        ))),
    }
}
