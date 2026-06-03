use crate::analyzer::env::Symbol;
use crate::analyzer::ty::TypeRegistry;
use crate::analyzer::typed_ast::TypedItem;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub const CACHE_VERSION: u8 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct PreludeCache {
    pub version: u8,
    pub source_hash: u64,
    pub typed_items: Vec<TypedItem>,
    pub registry: TypeRegistry,
    pub env_symbols: Vec<(String, Symbol)>,
}

pub fn compute_source_hash() -> u64 {
    let mut h = DefaultHasher::new();
    crate::stdlib::PRELUDE_SRC.hash(&mut h);
    crate::stdlib::AST_SRC.hash(&mut h);
    crate::stdlib::BUILTINS_SRC.hash(&mut h);
    crate::stdlib::INTERFACES_SRC.hash(&mut h);
    crate::stdlib::IMPLS_SRC.hash(&mut h);
    crate::stdlib::FUNCTIONS_SRC.hash(&mut h);
    h.finish()
}

pub fn cache_dir() -> std::path::PathBuf {
    // Explicit override wins.
    if let Ok(dir) = std::env::var("KILN_CACHE_DIR") {
        return std::path::PathBuf::from(dir);
    }

    #[cfg(target_os = "windows")]
    {
        // %LOCALAPPDATA%\kiln
        if let Ok(base) = std::env::var("LOCALAPPDATA") {
            return std::path::PathBuf::from(base).join("kiln");
        }
    }

    #[cfg(target_os = "macos")]
    {
        // ~/Library/Caches/kiln
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home)
                .join("Library")
                .join("Caches")
                .join("kiln");
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        // $XDG_CACHE_HOME/kiln  or  ~/.cache/kiln
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
            return std::path::PathBuf::from(xdg).join("kiln");
        }
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home).join(".cache").join("kiln");
        }
    }

    // Last-resort fallback.
    std::path::PathBuf::from(".kiln")
}

pub fn cache_path() -> std::path::PathBuf {
    cache_dir().join("prelude_cache_v1.bin")
}

pub fn load() -> Option<PreludeCache> {
    let path = cache_path();
    let bytes = std::fs::read(&path).ok()?;
    let (cache, _): (PreludeCache, usize) =
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).ok()?;
    if cache.version != CACHE_VERSION {
        return None;
    }
    if cache.source_hash != compute_source_hash() {
        return None;
    }
    Some(cache)
}

pub fn save(cache: &PreludeCache) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = bincode::serde::encode_to_vec(cache, bincode::config::standard()) {
        let _ = std::fs::write(&path, bytes);
    }
}
