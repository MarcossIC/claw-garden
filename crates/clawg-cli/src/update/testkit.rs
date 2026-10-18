//! Signs files with a key made for the test, the way the release does.

use std::io::Cursor;

use minisign::KeyPair;

pub(crate) struct Signer {
    /// The text of the `.pub` file.
    pub(crate) public: String,
    secret: minisign::SecretKey,
}

impl Signer {
    pub(crate) fn new() -> Self {
        let pair = KeyPair::generate_unencrypted_keypair().expect("key pair");
        let public = pair.pk.to_box().expect("public key box").into_string();
        Self { public, secret: pair.sk }
    }

    /// The text of the `.minisig` file.
    pub(crate) fn sign(&self, content: &[u8], trusted_comment: &str) -> String {
        minisign::sign(None, &self.secret, Cursor::new(content), Some(trusted_comment), None)
            .expect("sign")
            .into_string()
    }
}
