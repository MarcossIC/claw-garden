//! The minisign signature of a release's `SHA256SUMS`, checked against the
//! public keys this binary trusts.

use minisign_verify::{Error as VerifyError, PublicKey, Signature};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("a public key in the trusted list cannot be read: {0}")]
    BadKey(String),
    #[error("the signature file is not a minisign signature: {0}")]
    BadSignature(String),
    #[error("the signature was made by a key this binary does not trust")]
    UntrustedKey,
    #[error("the signature does not match the file: {0}")]
    Rejected(String),
}

/// The public keys a release signature is checked against, the text of a
/// `.pub` each. A `testing` build can take them from the environment; the
/// embedded list is empty, so any other build rejects every signature.
pub fn trusted_keys() -> Vec<String> {
    clawg_core::build_info::release_keys_override().unwrap_or_default()
}

/// Checks `signature` (the text of a `.minisig`) over `content` against each
/// of `keys` (the text of a `.pub` each) and returns the signed trusted
/// comment of the first key that made it.
///
/// A key that did not sign passes the turn to the next; any other failure
/// rejects at once. A signature in the legacy mode is always rejected: it
/// signs the content itself, where minisign now signs its hash.
pub fn verify(keys: &[&str], content: &[u8], signature: &str) -> Result<String, SignatureError> {
    let keys = keys
        .iter()
        .map(|key| PublicKey::decode(key).map_err(|e| SignatureError::BadKey(e.to_string())))
        .collect::<Result<Vec<_>, _>>()?;
    let signature =
        Signature::decode(signature).map_err(|e| SignatureError::BadSignature(e.to_string()))?;
    for key in &keys {
        match key.verify(content, &signature, false) {
            Ok(()) => return Ok(signature.trusted_comment().to_string()),
            Err(VerifyError::UnexpectedKeyId) => continue,
            Err(other) => return Err(SignatureError::Rejected(other.to_string())),
        }
    }
    Err(SignatureError::UntrustedKey)
}

#[cfg(test)]
mod signature_cases {
    use super::*;
    use crate::update::testkit::Signer;

    #[test]
    fn a_prehashed_signature_from_the_test_signer_is_accepted_without_legacy_mode() {
        let signer = Signer::new();
        let signature = signer.sign(b"sums", "clawg v1.2.3 SHA256SUMS");

        let comment = verify(&[&signer.public], b"sums", &signature).expect("accepted");

        assert_eq!(comment, "clawg v1.2.3 SHA256SUMS");
    }

    #[test]
    fn a_signature_by_the_second_key_of_the_list_is_accepted() {
        let (first, second) = (Signer::new(), Signer::new());
        let signature = second.sign(b"sums", "clawg v1.2.3 SHA256SUMS");

        assert!(verify(&[&first.public, &second.public], b"sums", &signature).is_ok());
    }

    #[test]
    fn a_key_outside_the_list_is_untrusted() {
        let (listed, outsider) = (Signer::new(), Signer::new());
        let signature = outsider.sign(b"sums", "clawg v1.2.3 SHA256SUMS");

        assert_eq!(
            verify(&[&listed.public], b"sums", &signature),
            Err(SignatureError::UntrustedKey)
        );
        assert_eq!(verify(&[], b"sums", &signature), Err(SignatureError::UntrustedKey));
    }

    #[test]
    fn an_altered_file_is_rejected() {
        let signer = Signer::new();
        let signature = signer.sign(b"sums", "clawg v1.2.3 SHA256SUMS");

        let result = verify(&[&signer.public], b"sumz", &signature);

        assert!(matches!(result, Err(SignatureError::Rejected(_))), "{result:?}");
    }

    #[test]
    fn the_signature_of_another_file_is_rejected() {
        let signer = Signer::new();
        let other = signer.sign(b"another file", "clawg v1.2.3 SHA256SUMS");

        let result = verify(&[&signer.public], b"sums", &other);

        assert!(matches!(result, Err(SignatureError::Rejected(_))), "{result:?}");
    }

    #[test]
    fn a_legacy_signature_is_rejected() {
        let signer = Signer::new();
        let signature = signer.sign(b"sums", "clawg v1.2.3 SHA256SUMS");

        let result = verify(&[&signer.public], b"sums", &as_legacy(&signature));

        assert!(matches!(result, Err(SignatureError::Rejected(_))), "{result:?}");
    }

    #[test]
    fn text_that_is_not_a_key_or_a_signature_is_named_as_such() {
        let signer = Signer::new();
        let signature = signer.sign(b"sums", "clawg v1.2.3 SHA256SUMS");

        assert!(matches!(
            verify(&["not a key"], b"sums", &signature),
            Err(SignatureError::BadKey(_))
        ));
        assert!(matches!(
            verify(&[&signer.public], b"sums", "not a signature"),
            Err(SignatureError::BadSignature(_))
        ));
    }

    #[test]
    fn the_files_signed_by_the_minisign_command_line_are_accepted() {
        let key = include_str!("testdata/fixture.pub");
        let sums = include_bytes!("testdata/SHA256SUMS");
        let signature = include_str!("testdata/SHA256SUMS.minisig");

        let comment = verify(&[key], sums, signature).expect("accepted");

        assert_eq!(comment, "clawg v0.19.0 SHA256SUMS");
    }

    /// The same signature with the algorithm field that marks the legacy mode
    /// (`Ed` instead of `ED`), the only difference the verifier reads first.
    fn as_legacy(signature: &str) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut lines: Vec<String> = signature.lines().map(String::from).collect();
        let head: Vec<u32> = lines[1][..4]
            .bytes()
            .map(|c| ALPHABET.iter().position(|&a| a == c).expect("base64") as u32)
            .collect();
        let bits = head.iter().fold(0u32, |acc, v| (acc << 6) | v);
        let mut bytes = [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8];
        assert_eq!(&bytes[..2], b"ED");
        bytes[1] = b'd';
        let bits = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        let encoded: String = (0..4)
            .map(|i| ALPHABET[((bits >> (18 - 6 * i)) & 63) as usize] as char)
            .collect();
        lines[1].replace_range(..4, &encoded);
        lines.join("\n") + "\n"
    }
}
