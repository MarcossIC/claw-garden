use std::path::Path;
use std::hash::{Hash, Hasher};
use siphasher::sip::SipHasher13;

/// Derive a stable, human-readable project_id from a project path.
///
/// Format: `{basename}-{hash4}` where `hash4` is 4 lowercase hex characters
/// of SipHash-1-3 (zero key) over the full path bytes (OS-string representation).
/// The result is deterministic on a given path, and the basename component
/// makes it human-readable without embedding parent directory information.
pub fn derive_project_id(project_path: &Path) -> String {
    let name = basename(project_path);
    let h = hash4(project_path.as_os_str().as_encoded_bytes());
    format!("{}-{}", name, h)
}

/// Return the 4-hex-character SipHash-1-3 fingerprint of `s`.
fn hash4(s: &[u8]) -> String {
    let mut hasher = SipHasher13::new_with_keys(0, 0);
    s.hash(&mut hasher);
    let h: u64 = hasher.finish();
    // Take the low 16 bits → 4 hex chars
    format!("{:04x}", h & 0xFFFF)
}

/// Return the file-name component of `p` as a lowercase string, falling back to
/// `"project"` if the path has no file-name or the name is not valid UTF-8.
fn basename(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Same path → same id.
    #[test]
    fn same_path_same_id() {
        let path = PathBuf::from("/home/user/projects/my-app");
        assert_eq!(derive_project_id(&path), derive_project_id(&path));
    }

    /// Format matches `{basename}-[0-9a-f]{4}`.
    #[test]
    fn format_matches_basename_hash4() {
        let path = PathBuf::from("/home/user/projects/my-app");
        let id = derive_project_id(&path);
        // Must start with the basename (lowercased)
        assert!(
            id.starts_with("my-app-"),
            "Expected id to start with 'my-app-', got: {}",
            id
        );
        // Must end with exactly 4 hex chars
        let hash_part = id.split('-').last().unwrap();
        assert_eq!(hash_part.len(), 4, "Hash part must be 4 chars: {}", id);
        assert!(
            hash_part.chars().all(|c| c.is_ascii_hexdigit()),
            "Hash part must be hex: {}",
            id
        );
    }

    /// Different paths → different ids.
    #[test]
    fn different_paths_different_ids() {
        let p1 = PathBuf::from("/home/user/projects/my-app");
        let p2 = PathBuf::from("/home/user/projects/my-app-v2");
        assert_ne!(derive_project_id(&p1), derive_project_id(&p2));
    }
}
