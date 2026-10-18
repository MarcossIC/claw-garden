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
    use minisign::KeyPair;
    use std::io::Cursor;

    struct Signer {
        public: String,
        secret: minisign::SecretKey,
    }

    impl Signer {
        fn new() -> Self {
            let pair = KeyPair::generate_unencrypted_keypair().expect("key pair");
            let public = pair.pk.to_box().expect("public key box").into_string();
            Self { public, secret: pair.sk }
        }

        fn sign(&self, content: &[u8], trusted_comment: &str) -> String {
            minisign::sign(None, &self.secret, Cursor::new(content), Some(trusted_comment), None)
                .expect("sign")
                .into_string()
        }
    }

    #[test]
    fn a_prehashed_signature_from_the_test_signer_is_accepted_without_legacy_mode() {
        let signer = Signer::new();
        let signature = signer.sign(b"sums", "clawg v1.2.3 SHA256SUMS");

        let comment = verify(&[&signer.public], b"sums", &signature).expect("accepted");

        assert_eq!(comment, "clawg v1.2.3 SHA256SUMS");
    }
}
