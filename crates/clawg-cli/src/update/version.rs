//! Release versions: `X.Y.Z` and nothing else.

use std::fmt;

/// Three decimal numbers. The field order is the comparison order, so the
/// derived `Ord` is the numeric one (`0.9.0 < 0.18.0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0:?} is not a version of the form X.Y.Z")]
pub struct VersionError(String);

impl Version {
    /// `X.Y.Z`: three numbers of plain digits, with no leading zero but a lone
    /// `0`, and no prefix, suffix or space.
    pub fn parse(text: &str) -> Result<Self, VersionError> {
        let malformed = || VersionError(text.to_string());
        let mut parts = text.split('.');
        let mut next = || -> Result<u64, VersionError> {
            let part = parts.next().ok_or_else(malformed)?;
            let plain = !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            if !plain {
                return Err(malformed());
            }
            part.parse().map_err(|_| malformed())
        };
        let version = Version { major: next()?, minor: next()?, patch: next()? };
        if parts.next().is_some() {
            return Err(malformed());
        }
        Ok(version)
    }

    /// The version a release tag names: `v` followed by [`Version::parse`].
    pub fn from_tag(tag: &str) -> Result<Self, VersionError> {
        match tag.strip_prefix('v') {
            Some(version) => Self::parse(version).map_err(|_| VersionError(tag.to_string())),
            None => Err(VersionError(tag.to_string())),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod version_parse_and_order_cases {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).expect("a version")
    }

    #[test]
    fn three_numbers_parse_and_print_back() {
        assert_eq!(v("0.19.0"), Version { major: 0, minor: 19, patch: 0 });
        assert_eq!(v("10.2.30").to_string(), "10.2.30");
    }

    #[test]
    fn anything_else_is_refused() {
        for text in [
            "", "0", "0.19", "0.19.0.1", "v0.19.0", "0.19.0-rc1", "0.19.0+build", "01.2.3", "1.02.3",
            "1.2.03", "1.2.-3", "+1.2.3", "1.2.x", " 1.2.3", "1.2.3 ", "1..3", "99999999999999999999.0.0",
        ] {
            assert!(Version::parse(text).is_err(), "{text:?} must be refused");
        }
    }

    #[test]
    fn a_tag_is_a_version_with_a_leading_v() {
        assert_eq!(Version::from_tag("v0.19.0"), Ok(v("0.19.0")));
        for tag in ["0.19.0", "V0.19.0", "v0.19", "vv0.19.0", "v"] {
            assert!(Version::from_tag(tag).is_err(), "{tag:?} must be refused");
        }
    }

    #[test]
    fn the_order_is_numeric_not_textual() {
        assert!(v("0.9.0") < v("0.18.0"));
        assert!(v("0.18.0") < v("0.18.1"));
        assert!(v("0.18.9") < v("0.19.0"));
        assert!(v("0.99.99") < v("1.0.0"));
        assert_eq!(v("0.18.0"), v("0.18.0"));
    }

    #[test]
    fn only_a_strictly_greater_version_is_an_update() {
        let installed = v("0.18.0");
        assert!(v("0.19.0") > installed);
        assert!(!(v("0.18.0") > installed));
        assert!(!(v("0.17.9") > installed));
    }
}
