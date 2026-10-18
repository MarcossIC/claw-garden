//! Which release to offer, and which of its files is this binary's.

use serde::Deserialize;

use super::version::Version;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReleaseError {
    #[error("the release listing is not what GitHub documents: {0}")]
    Listing(String),
    #[error("release {tag} has no file for {triple}: expected {asset}")]
    MissingAsset { tag: String, triple: String, asset: String },
}

/// A release the update may offer: published, with a tag `vX.Y.Z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    assets: Vec<String>,
}

#[derive(Deserialize)]
struct Listed {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ListedAsset>,
}

#[derive(Deserialize)]
struct ListedAsset {
    name: String,
}

/// The file of the `clawg` executable built for `triple`.
pub fn asset_name(triple: &str) -> String {
    let suffix = if triple.ends_with("-windows-msvc") { ".exe" } else { "" };
    format!("clawg-{triple}{suffix}")
}

/// The newest of the releases in the body of `GET /repos/{owner}/{repo}/releases`,
/// leaving out drafts, prereleases and tags that are not `vX.Y.Z`. The newest
/// is decided by version, not by position or date.
pub fn choose(listing: &str) -> Result<Option<Release>, ReleaseError> {
    let listed: Vec<Listed> =
        serde_json::from_str(listing).map_err(|e| ReleaseError::Listing(e.to_string()))?;
    Ok(listed
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let version = Version::from_tag(&r.tag_name).ok()?;
            let assets = r.assets.into_iter().map(|a| a.name).collect();
            Some(Release { version, tag: r.tag_name, assets })
        })
        .max_by_key(|r| r.version))
}

impl Release {
    /// The file of the executable for `triple`; the error names the triple.
    pub fn binary_asset(&self, triple: &str) -> Result<String, ReleaseError> {
        let asset = asset_name(triple);
        if self.assets.contains(&asset) {
            return Ok(asset);
        }
        Err(ReleaseError::MissingAsset { tag: self.tag.clone(), triple: triple.to_string(), asset })
    }
}

#[cfg(test)]
mod release_choice_and_asset_cases {
    use super::*;

    const WINDOWS: &str = "x86_64-pc-windows-msvc";
    const LINUX: &str = "x86_64-unknown-linux-gnu";

    fn release(tag: &str, draft: bool, prerelease: bool, assets: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "draft": draft,
            "prerelease": prerelease,
            "name": "ignored",
            "assets": assets.iter().map(|name| serde_json::json!({ "name": name, "size": 1 })).collect::<Vec<_>>(),
        })
    }

    fn listing(releases: &[serde_json::Value]) -> String {
        serde_json::Value::Array(releases.to_vec()).to_string()
    }

    const FILES: [&str; 5] = [
        "clawg-x86_64-pc-windows-msvc.exe",
        "clawg-x86_64-unknown-linux-gnu",
        "install.ps1",
        "SHA256SUMS",
        "SHA256SUMS.minisig",
    ];

    #[test]
    fn the_asset_is_named_by_triple_and_windows_gets_exe() {
        assert_eq!(asset_name(WINDOWS), "clawg-x86_64-pc-windows-msvc.exe");
        assert_eq!(asset_name(LINUX), "clawg-x86_64-unknown-linux-gnu");
    }

    #[test]
    fn a_draft_and_a_prerelease_newer_than_the_last_release_are_skipped() {
        let json = listing(&[
            release("v0.21.0", true, false, &FILES),
            release("v0.20.0", false, true, &FILES),
            release("v0.19.0", false, false, &FILES),
            release("v0.18.0", false, false, &FILES),
        ]);

        let chosen = choose(&json).expect("valid listing").expect("a release");

        assert_eq!(chosen.version.to_string(), "0.19.0");
        assert_eq!(chosen.tag, "v0.19.0");
    }

    #[test]
    fn the_greatest_version_wins_whatever_the_order_of_the_listing() {
        let json = listing(&[
            release("v0.9.0", false, false, &FILES),
            release("v0.18.0", false, false, &FILES),
            release("v0.17.9", false, false, &FILES),
        ]);

        let chosen = choose(&json).unwrap().unwrap();

        assert_eq!(chosen.tag, "v0.18.0");
    }

    #[test]
    fn a_tag_that_is_not_a_version_is_skipped() {
        let json = listing(&[
            release("nightly", false, false, &FILES),
            release("v0.20", false, false, &FILES),
            release("0.30.0", false, false, &FILES),
            release("v0.19.0-rc1", false, false, &FILES),
            release("v0.18.0", false, false, &FILES),
        ]);

        assert_eq!(choose(&json).unwrap().unwrap().tag, "v0.18.0");
    }

    #[test]
    fn no_published_release_is_not_an_error() {
        assert_eq!(choose("[]").unwrap(), None);
        let only_drafts = listing(&[release("v0.19.0", true, false, &FILES)]);
        assert_eq!(choose(&only_drafts).unwrap(), None);
    }

    #[test]
    fn a_listing_that_is_not_json_of_releases_is_an_error() {
        for text in ["", "not json", "{}", "[{\"tag_name\": 3}]"] {
            assert!(matches!(choose(text), Err(ReleaseError::Listing(_))), "{text:?}");
        }
    }

    #[test]
    fn the_own_asset_is_found_by_triple() {
        let json = listing(&[release("v0.19.0", false, false, &FILES)]);
        let chosen = choose(&json).unwrap().unwrap();

        assert_eq!(chosen.binary_asset(WINDOWS).unwrap(), "clawg-x86_64-pc-windows-msvc.exe");
        assert_eq!(chosen.binary_asset(LINUX).unwrap(), "clawg-x86_64-unknown-linux-gnu");
    }

    #[test]
    fn a_release_without_the_own_asset_names_the_triple() {
        let json = listing(&[release("v0.19.0", false, false, &["clawg-x86_64-pc-windows-msvc.exe"])]);
        let chosen = choose(&json).unwrap().unwrap();

        let error = chosen.binary_asset(LINUX).unwrap_err();

        assert!(error.to_string().contains(LINUX), "{error}");
        assert!(error.to_string().contains("v0.19.0"), "{error}");
    }
}
