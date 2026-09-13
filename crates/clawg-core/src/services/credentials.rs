use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::services::oauth::OAuthTokenSet;

/// Path: `<data_dir>/credentials.json`.
fn credentials_path() -> PathBuf {
    crate::paths::data_dir_from_env()
        .unwrap_or_else(|e| panic!("failed to resolve data directory: {e}"))
        .join("credentials.json")
}

/// Load OAuth token set from credentials file.
pub fn load_oauth_credentials() -> Option<OAuthTokenSet> {
    let path = credentials_path();
    let content = fs::read_to_string(&path).ok()?;
    let map: HashMap<String, serde_json::Value> = serde_json::from_str(&content).ok()?;
    let oauth_value = map.get("oauth")?;
    serde_json::from_value(oauth_value.clone()).ok()
}

/// Save OAuth token set to credentials file.
/// Preserves other keys in the file (atomic write via temp file).
pub fn save_oauth_credentials(token_set: &OAuthTokenSet) -> Result<(), std::io::Error> {
    let path = credentials_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Read existing content to preserve other keys
    let mut map: HashMap<String, serde_json::Value> = if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let oauth_value = serde_json::to_value(token_set)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    map.insert("oauth".to_string(), oauth_value);

    let json = serde_json::to_string_pretty(&map)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // Atomic write: temp file + rename
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &json)?;
    fs::rename(&tmp_path, &path)?;

    // Restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Remove OAuth credentials (logout).
pub fn clear_oauth_credentials() -> Result<(), std::io::Error> {
    let path = credentials_path();
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&path)?;
    let mut map: HashMap<String, serde_json::Value> =
        serde_json::from_str(&content).unwrap_or_default();
    map.remove("oauth");

    if map.is_empty() {
        fs::remove_file(&path)?;
    } else {
        let json = serde_json::to_string_pretty(&map)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        fs::write(&path, json)?;
    }

    Ok(())
}
