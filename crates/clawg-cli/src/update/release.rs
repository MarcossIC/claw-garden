//! Which release to offer, and which of its files is this binary's.

use serde::Deserialize;

use super::version::Version;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReleaseError {
    #[error("the release listing is not what GitHub documents: {0}")]
    Listing(String),
    #[error("release {tag} has no file for {triple}: expected {asset}")]
    MissingAsset { tag: String, triple: String, asset: String },
    #[error("line {number} of SHA256SUMS is not a 64-digit lowercase hash, a separator and a name")]
    BadSumsLine { number: usize },
    #[error("SHA256SUMS lists {0} twice")]
    DuplicateSumsName(String),
    #[error("SHA256SUMS has no line for {0}")]
    NotInSums(String),
    #[error("SHA256SUMS is signed as {signed:?}, not for this release ({expected:?})")]
    SignedForAnotherVersion { signed: String, expected: String },
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

/// The hashes a release publishes in `SHA256SUMS`, as `sha256sum` writes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256Sums(Vec<(String, String)>);

impl Sha256Sums {
    /// Lines of `<64 lowercase hex digits><space><space or *><name>`. Any other
    /// line, blank ones included, and a name listed twice are refused.
    pub fn parse(text: &str) -> Result<Self, ReleaseError> {
        let mut entries: Vec<(String, String)> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let bad = || ReleaseError::BadSumsLine { number: index + 1 };
            let (hash, rest) = (line.get(..64).ok_or_else(bad)?, &line[64..]);
            let lowercase_hex = hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
            let name = rest
                .strip_prefix("  ")
                .or_else(|| rest.strip_prefix(" *"))
                .filter(|name| !name.is_empty());
            let (true, Some(name)) = (lowercase_hex, name) else {
                return Err(bad());
            };
            if entries.iter().any(|(known, _)| known == name) {
                return Err(ReleaseError::DuplicateSumsName(name.to_string()));
            }
            entries.push((name.to_string(), hash.to_string()));
        }
        Ok(Self(entries))
    }

    pub fn hash_of(&self, name: &str) -> Result<&str, ReleaseError> {
        self.0
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, hash)| hash.as_str())
            .ok_or_else(|| ReleaseError::NotInSums(name.to_string()))
    }
}

/// The trusted comment the release signs `SHA256SUMS` with, which binds the
/// signed hashes to the version the listing announced.
pub fn signed_comment(version: Version) -> String {
    format!("clawg v{version} SHA256SUMS")
}

pub fn check_signed_comment(signed: &str, version: Version) -> Result<(), ReleaseError> {
    let expected = signed_comment(version);
    if signed == expected {
        return Ok(());
    }
    Err(ReleaseError::SignedForAnotherVersion { signed: signed.to_string(), expected })
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

#[cfg(test)]
mod sha256sums_parse_cases {
    use super::*;

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn the_lines_sha256sum_writes_are_read_in_both_modes() {
        let text = format!("{HASH_A}  clawg-x86_64-unknown-linux-gnu
{HASH_B} *install.ps1
");

        let sums = Sha256Sums::parse(&text).expect("valid");

        assert_eq!(sums.hash_of("clawg-x86_64-unknown-linux-gnu").unwrap(), HASH_A);
        assert_eq!(sums.hash_of("install.ps1").unwrap(), HASH_B);
    }

    #[test]
    fn a_missing_line_for_the_asset_is_its_own_error() {
        let sums = Sha256Sums::parse(&format!("{HASH_A}  install.ps1
")).unwrap();

        assert_eq!(
            sums.hash_of("clawg-x86_64-unknown-linux-gnu"),
            Err(ReleaseError::NotInSums("clawg-x86_64-unknown-linux-gnu".into()))
        );
    }

    #[test]
    fn a_hash_that_is_not_64_lowercase_hex_digits_names_its_line() {
        let short = format!("{HASH_A}  a
{}  b
", &HASH_B[..63]);
        let upper = format!("{}  a
", HASH_B.to_uppercase());
        let not_hex = format!("{}z  a
", &HASH_B[..63]);
        let long = format!("{HASH_B}0  a
");

        assert_eq!(Sha256Sums::parse(&short), Err(ReleaseError::BadSumsLine { number: 2 }));
        for text in [upper, not_hex, long] {
            assert_eq!(Sha256Sums::parse(&text), Err(ReleaseError::BadSumsLine { number: 1 }), "{text}");
        }
    }

    #[test]
    fn a_line_that_is_not_hash_separator_name_names_its_line() {
        for text in [
            format!("{HASH_A} a
"),
            format!("{HASH_A}  
"),
            format!("{HASH_A}
"),
            format!("
{HASH_A}  a
"),
            "no hash here
".to_string(),
        ] {
            assert!(matches!(Sha256Sums::parse(&text), Err(ReleaseError::BadSumsLine { .. })), "{text:?}");
        }
    }

    #[test]
    fn a_repeated_name_is_refused() {
        let text = format!("{HASH_A}  install.ps1
{HASH_B}  install.ps1
");

        assert_eq!(Sha256Sums::parse(&text), Err(ReleaseError::DuplicateSumsName("install.ps1".into())));
    }

    #[test]
    fn the_three_failures_are_three_different_errors() {
        let short = Sha256Sums::parse(&format!("{}  a
", &HASH_A[..10])).unwrap_err();
        let twice = Sha256Sums::parse(&format!("{HASH_A}  a
{HASH_A}  a
")).unwrap_err();
        let missing = Sha256Sums::parse(&format!("{HASH_A}  a
")).unwrap().hash_of("b").unwrap_err();

        assert_ne!(short, twice);
        assert_ne!(short, missing);
        assert_ne!(twice, missing);
    }
}

#[cfg(test)]
mod trusted_comment_cases {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn the_signed_comment_must_name_the_announced_version() {
        assert_eq!(signed_comment(v("0.19.0")), "clawg v0.19.0 SHA256SUMS");
        assert_eq!(check_signed_comment("clawg v0.19.0 SHA256SUMS", v("0.19.0")), Ok(()));
    }

    #[test]
    fn the_comment_of_another_version_is_refused() {
        assert_eq!(
            check_signed_comment("clawg v0.18.0 SHA256SUMS", v("0.19.0")),
            Err(ReleaseError::SignedForAnotherVersion {
                signed: "clawg v0.18.0 SHA256SUMS".into(),
                expected: "clawg v0.19.0 SHA256SUMS".into(),
            })
        );
        for comment in ["", "timestamp:1", "clawg v0.19.0 SHA256SUMS ", "clawg v0.19.0"] {
            assert!(check_signed_comment(comment, v("0.19.0")).is_err(), "{comment:?}");
        }
    }
}
