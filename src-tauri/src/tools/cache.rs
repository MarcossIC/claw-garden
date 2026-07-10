use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime};

use dashmap::DashMap;
use once_cell::sync::Lazy;
use serde_json::Value;

use super::CachePolicy;

const DEFAULT_MAX_BYTES: usize = 50 * 1024 * 1024; // 50 MB

struct CacheEntry {
    result: String,
    cached_at: Instant,
    last_accessed: Instant,
    path: Option<PathBuf>,
    mtime: Option<SystemTime>,
    policy: CachePolicy,
    /// Paths this result "touches" — used for write-invalidation.
    touched_paths: Vec<PathBuf>,
    size_bytes: usize,
}

pub struct ToolResultCache {
    map: DashMap<u64, CacheEntry>,
    max_bytes: usize,
    used_bytes: AtomicUsize,
}

impl ToolResultCache {
    pub fn new() -> Self {
        Self::with_max_bytes(DEFAULT_MAX_BYTES)
    }

    pub fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            map: DashMap::new(),
            max_bytes,
            used_bytes: AtomicUsize::new(0),
        }
    }

    /// Look up a cached result. Returns Some if valid, None if miss/stale.
    pub fn lookup(&self, tool_name: &str, input: &Value) -> Option<String> {
        let key = make_cache_key(tool_name, input);
        let mut entry = self.map.get_mut(&key)?;

        let valid = match &entry.policy {
            CachePolicy::Mtime => {
                if let Some(path) = &entry.path {
                    if let Ok(meta) = std::fs::metadata(path) {
                        if let Ok(current_mtime) = meta.modified() {
                            entry.mtime.map_or(false, |m| m == current_mtime)
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CachePolicy::Ttl(ttl) => entry.cached_at.elapsed() <= *ttl,
        };

        if !valid {
            drop(entry);
            if let Some((_, e)) = self.map.remove(&key) {
                self.used_bytes.fetch_sub(e.size_bytes, Ordering::Relaxed);
            }
            return None;
        }

        entry.last_accessed = Instant::now();
        Some(entry.result.clone())
    }

    /// Store a result in the cache.
    pub fn store(
        &self,
        tool_name: &str,
        input: &Value,
        result: String,
        policy: CachePolicy,
        path: Option<PathBuf>,
        touched_paths: Vec<PathBuf>,
    ) {
        let key = make_cache_key(tool_name, input);
        let size = result.len();

        // Evict if over limit
        if self.used_bytes.load(Ordering::Relaxed) + size > self.max_bytes {
            self.evict_lru();
        }

        let mtime = path.as_ref().and_then(|p| {
            std::fs::metadata(p).ok()?.modified().ok()
        });

        self.used_bytes.fetch_add(size, Ordering::Relaxed);
        self.map.insert(key, CacheEntry {
            result,
            cached_at: Instant::now(),
            last_accessed: Instant::now(),
            path,
            mtime,
            policy,
            touched_paths,
            size_bytes: size,
        });
    }

    /// Invalidate all cache entries whose touched_paths include the written path.
    /// Called after write_file / edit_file succeed.
    /// Best-effort — never fails the write operation.
    pub fn invalidate_touching(&self, written_path: &Path) {
        let canonical = written_path.canonicalize().unwrap_or_else(|_| written_path.to_owned());
        let mut to_remove = Vec::new();
        for entry in self.map.iter() {
            let touches = entry.touched_paths.iter().any(|p| {
                p.canonicalize().unwrap_or_else(|_| p.clone()) == canonical
            });
            if touches {
                to_remove.push(*entry.key());
            }
        }
        for key in to_remove {
            if let Some((_, e)) = self.map.remove(&key) {
                self.used_bytes.fetch_sub(e.size_bytes, Ordering::Relaxed);
            }
        }
    }

    fn evict_lru(&self) {
        // Find the least recently accessed entry and remove it.
        let oldest_key = self.map
            .iter()
            .min_by_key(|e| e.last_accessed)
            .map(|e| *e.key());
        if let Some(key) = oldest_key {
            if let Some((_, e)) = self.map.remove(&key) {
                self.used_bytes.fetch_sub(e.size_bytes, Ordering::Relaxed);
            }
        }
    }
}

impl Default for ToolResultCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Global cache shared across all agents.
pub static TOOL_CACHE: Lazy<ToolResultCache> = Lazy::new(ToolResultCache::new);

/// Normalize JSON input before hashing: sort keys, omit nulls.
fn normalize_json(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut pairs: Vec<_> = map.iter()
                .filter(|(_, v)| !v.is_null())
                .collect();
            pairs.sort_by_key(|(k, _)| k.as_str());
            let normalized: serde_json::Map<String, Value> = pairs
                .into_iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            serde_json::to_string(&Value::Object(normalized)).unwrap_or_default()
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

pub fn make_cache_key(tool_name: &str, input: &Value) -> u64 {
    let normalized = normalize_json(input);
    let mut h = DefaultHasher::new();
    tool_name.hash(&mut h);
    normalized.hash(&mut h);
    h.finish()
}
