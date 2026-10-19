//! Asks the release server for what `clawg update` needs, and only when the
//! user runs it.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::release::{check_signed_comment, Release, ReleaseError, Sha256Sums};
use super::signature::{verify, SignatureError};

const REPOSITORY: &str = "MarcossIC/claw-garden";
const GITHUB_API: &str = "https://api.github.com";
const GITHUB_DOWNLOADS: &str = "https://github.com";

/// What each kind of answer may weigh: the listing, the signed hashes, their
/// signature and the executable.
pub const LISTING_CAP: u64 = 1024 * 1024;
pub const SUMS_CAP: u64 = 64 * 1024;
pub const SIGNATURE_CAP: u64 = 4 * 1024;
pub const BINARY_CAP: u64 = 256 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("cannot reach {url}: {reason}")]
    Request { url: String, reason: String },
    #[error("{url} answered {status}")]
    Status { url: String, status: u16 },
    /// GitHub takes a few dozen requests an hour from an address without a
    /// token, and a shared one can use them up before this user asks.
    #[error(
        "{url} answered {status}: the GitHub API's rate limit for this address is used up; {}",
        try_again(.resets_in_minutes)
    )]
    RateLimited { url: String, status: u16, resets_in_minutes: Option<u64> },
    #[error("{url} is larger than the {cap} bytes allowed")]
    TooLarge { url: String, cap: u64 },
    #[error("{url} is not text")]
    NotText { url: String },
    #[error("cannot write {path}: {reason}")]
    Write { path: String, reason: String },
    #[error(transparent)]
    Release(#[from] ReleaseError),
    #[error(transparent)]
    Signature(#[from] SignatureError),
    #[error("{asset} does not match the signed hash: expected {expected}, got {actual}")]
    HashMismatch { asset: String, expected: String, actual: String },
}

/// A release server: GitHub's, or in a `testing` build the one the
/// environment names.
pub struct ReleaseClient {
    http: reqwest::Client,
    api_base: String,
    downloads_base: String,
}

impl ReleaseClient {
    pub fn new(api_base: &str, downloads_base: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(format!("clawg/{}", clawg_core::build_info::version()))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .expect("a client with a user agent and timeouts builds");
        Self {
            http,
            api_base: api_base.trim_end_matches('/').to_string(),
            downloads_base: downloads_base.trim_end_matches('/').to_string(),
        }
    }

    pub fn github() -> Self {
        Self::new(GITHUB_API, GITHUB_DOWNLOADS)
    }

    /// GitHub's, unless a `testing` build was told another server.
    pub fn for_this_build() -> Self {
        match clawg_core::build_info::release_url_override() {
            Some(base) => Self::new(&base, &base),
            None => Self::github(),
        }
    }

    pub fn releases_url(&self) -> String {
        format!("{}/repos/{REPOSITORY}/releases?per_page=30", self.api_base)
    }

    pub fn asset_url(&self, tag: &str, asset: &str) -> String {
        format!("{}/{REPOSITORY}/releases/download/{tag}/{asset}", self.downloads_base)
    }

    /// The body of the releases listing, newest first.
    pub async fn list_releases(&self) -> Result<String, FetchError> {
        let url = self.releases_url();
        text(&url, self.get_capped(&url, LISTING_CAP).await?)
    }

    /// Downloads the executable of `release` for `triple` into `dest`, a file
    /// that must not exist yet, and leaves nothing there unless it is the
    /// file the signed hashes name.
    ///
    /// The hashes come first, with their signature, and the executable is not
    /// asked for until both have been checked.
    pub async fn fetch_binary(
        &self,
        release: &Release,
        triple: &str,
        keys: &[&str],
        dest: &Path,
    ) -> Result<(), FetchError> {
        let asset = release.binary_asset(triple)?;
        let sums_url = self.asset_url(&release.tag, "SHA256SUMS");
        let sums = self.get_capped(&sums_url, SUMS_CAP).await?;
        let signature_url = self.asset_url(&release.tag, "SHA256SUMS.minisig");
        let signature = text(&signature_url, self.get_capped(&signature_url, SIGNATURE_CAP).await?)?;

        let signed_as = verify(keys, &sums, &signature)?;
        check_signed_comment(&signed_as, release.version)?;
        let sums = Sha256Sums::parse(&text(&sums_url, sums)?)?;
        let expected = sums.hash_of(&asset)?.to_string();

        let actual = self.download_hashed(&self.asset_url(&release.tag, &asset), BINARY_CAP, dest).await?;
        if actual != expected {
            let _ = std::fs::remove_file(dest);
            return Err(FetchError::HashMismatch { asset, expected, actual });
        }
        Ok(())
    }

    async fn get_capped(&self, url: &str, cap: u64) -> Result<Vec<u8>, FetchError> {
        let mut response = self.open(url, cap).await?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| request_failed(url, e))? {
            if body.len() as u64 + chunk.len() as u64 > cap {
                return Err(FetchError::TooLarge { url: url.to_string(), cap });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// Streams `url` into the new file `dest` and returns the lowercase hex
    /// sha256 of what was written. The file is removed on any failure.
    async fn download_hashed(&self, url: &str, cap: u64, dest: &Path) -> Result<String, FetchError> {
        let mut response = self.open(url, cap).await?;
        let write_failed = |e: std::io::Error| FetchError::Write {
            path: dest.display().to_string(),
            reason: e.to_string(),
        };
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(dest).map_err(write_failed)?;
        let mut hash = Sha256::new();
        let mut written = 0u64;
        let streamed = async {
            while let Some(chunk) = response.chunk().await.map_err(|e| request_failed(url, e))? {
                written += chunk.len() as u64;
                if written > cap {
                    return Err(FetchError::TooLarge { url: url.to_string(), cap });
                }
                hash.update(&chunk);
                file.write_all(&chunk).map_err(write_failed)?;
            }
            // On disk before it is renamed into place, or a power cut can leave
            // the installed name on an empty file; `flush` does nothing on a `File`.
            file.sync_all().map_err(write_failed)
        }
        .await;
        drop(file);
        if let Err(e) = streamed {
            let _ = std::fs::remove_file(dest);
            return Err(e);
        }
        Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    async fn open(&self, url: &str, cap: u64) -> Result<reqwest::Response, FetchError> {
        let response = self
            .http
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| request_failed(url, e))?;
        if !response.status().is_success() {
            return Err(refusal(
                url,
                response.status().as_u16(),
                header(&response, "x-ratelimit-remaining"),
                header(&response, "x-ratelimit-reset"),
                now_secs(),
            ));
        }
        if response.content_length().is_some_and(|len| len > cap) {
            return Err(FetchError::TooLarge { url: url.to_string(), cap });
        }
        Ok(response)
    }
}

/// The error for an answer that is not a success. GitHub's rate limit comes
/// as a 403 or a 429 with no requests left, and the time it resets at in
/// seconds since the epoch.
fn refusal(url: &str, status: u16, remaining: Option<&str>, reset: Option<&str>, now_secs: u64) -> FetchError {
    let url = url.to_string();
    if !matches!(status, 403 | 429) || remaining.map(str::trim) != Some("0") {
        return FetchError::Status { url, status };
    }
    let resets_in_minutes = reset
        .and_then(|at| at.trim().parse::<u64>().ok())
        .map(|at| at.saturating_sub(now_secs).div_ceil(60));
    FetchError::RateLimited { url, status, resets_in_minutes }
}

fn try_again(resets_in_minutes: &Option<u64>) -> String {
    match resets_in_minutes {
        Some(0) => "try again now".to_string(),
        Some(minutes) => format!("try again in {minutes} min"),
        None => "try again within the hour".to_string(),
    }
}

fn header<'a>(response: &'a reqwest::Response, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|value| value.to_str().ok())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn request_failed(url: &str, error: reqwest::Error) -> FetchError {
    FetchError::Request { url: url.to_string(), reason: error.without_url().to_string() }
}

fn text(url: &str, body: Vec<u8>) -> Result<String, FetchError> {
    String::from_utf8(body).map_err(|_| FetchError::NotText { url: url.to_string() })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode, Uri};
    use axum::Router;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::update::release::choose;
    use crate::update::testkit::Signer;

    const TRIPLE: &str = "x86_64-unknown-linux-gnu";
    const ASSET: &str = "clawg-x86_64-unknown-linux-gnu";
    const BINARY: &[u8] = b"the new executable";

    #[derive(Default)]
    struct Served {
        files: HashMap<String, Vec<u8>>,
        requests: Vec<(String, Option<String>)>,
    }

    /// A release server on an ephemeral port: it answers the paths in `files`
    /// and remembers what it was asked and with which `User-Agent`.
    struct Server {
        base: String,
        served: Arc<Mutex<Served>>,
    }

    impl Server {
        async fn start(files: &[(&str, Vec<u8>)]) -> Self {
            let served = Arc::new(Mutex::new(Served {
                files: files.iter().map(|(path, body)| (path.to_string(), body.clone())).collect(),
                requests: Vec::new(),
            }));
            async fn answer(
                State(served): State<Arc<Mutex<Served>>>,
                headers: HeaderMap,
                uri: Uri,
            ) -> Result<Vec<u8>, StatusCode> {
                let agent = headers.get("user-agent").and_then(|v| v.to_str().ok()).map(String::from);
                let mut served = served.lock().unwrap();
                let path = uri.path().to_string();
                served.requests.push((path.clone(), agent));
                served.files.get(&path).cloned().ok_or(StatusCode::NOT_FOUND)
            }
            let app = Router::new().fallback(answer).with_state(served.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            Self { base, served }
        }

        fn client(&self) -> ReleaseClient {
            ReleaseClient::new(&self.base, &self.base)
        }

        fn requested(&self) -> Vec<String> {
            self.served.lock().unwrap().requests.iter().map(|(path, _)| path.clone()).collect()
        }

        fn agents(&self) -> Vec<Option<String>> {
            self.served.lock().unwrap().requests.iter().map(|(_, agent)| agent.clone()).collect()
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn release_dir(tag: &str, file: &str) -> String {
        format!("/MarcossIC/claw-garden/releases/download/{tag}/{file}")
    }

    /// The files of release `tag` with `sums` signed by `signer` as
    /// `comment`, and a binary of `binary`.
    fn release_files(signer: &Signer, tag: &str, comment: &str, binary: &[u8]) -> Vec<(String, Vec<u8>)> {
        let sums = format!("{}  {ASSET}\n", sha256_hex(binary));
        vec![
            (release_dir(tag, "SHA256SUMS"), sums.clone().into_bytes()),
            (release_dir(tag, "SHA256SUMS.minisig"), signer.sign(sums.as_bytes(), comment).into_bytes()),
            (release_dir(tag, ASSET), binary.to_vec()),
        ]
    }

    async fn serve(files: Vec<(String, Vec<u8>)>) -> Server {
        let borrowed: Vec<(&str, Vec<u8>)> = files.iter().map(|(p, b)| (p.as_str(), b.clone())).collect();
        Server::start(&borrowed).await
    }

    fn release(tag: &str) -> Release {
        let listing = serde_json::json!([{
            "tag_name": tag, "draft": false, "prerelease": false,
            "assets": [{ "name": "SHA256SUMS" }, { "name": "SHA256SUMS.minisig" }, { "name": ASSET }],
        }]);
        choose(&listing.to_string()).unwrap().unwrap()
    }

    #[tokio::test]
    async fn requests_carry_a_user_agent() {
        let signer = Signer::new();
        let server = serve(release_files(&signer, "v0.19.0", "clawg v0.19.0 SHA256SUMS", BINARY)).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("candidate");

        let _ = server.client().list_releases().await;
        server
            .client()
            .fetch_binary(&release("v0.19.0"), TRIPLE, &[&signer.public], &dest)
            .await
            .expect("fetched");

        let agents = server.agents();
        assert_eq!(agents.len(), 4, "{:?}", server.requested());
        let expected = format!("clawg/{}", clawg_core::build_info::version());
        assert!(agents.iter().all(|a| a.as_deref() == Some(expected.as_str())), "{agents:?}");
    }

    #[tokio::test]
    async fn the_listing_is_asked_with_the_page_size_and_returned_as_text() {
        let server = Server::start(&[("/repos/MarcossIC/claw-garden/releases", b"[]".to_vec())]).await;

        let body = server.client().list_releases().await.expect("listing");

        assert_eq!(body, "[]");
        assert_eq!(server.requested(), ["/repos/MarcossIC/claw-garden/releases"]);
    }

    #[tokio::test]
    async fn the_signed_hashes_come_first_and_the_binary_last() {
        let signer = Signer::new();
        let server = serve(release_files(&signer, "v0.19.0", "clawg v0.19.0 SHA256SUMS", BINARY)).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("candidate");

        server
            .client()
            .fetch_binary(&release("v0.19.0"), TRIPLE, &[&signer.public], &dest)
            .await
            .expect("fetched");

        assert_eq!(
            server.requested(),
            [
                release_dir("v0.19.0", "SHA256SUMS"),
                release_dir("v0.19.0", "SHA256SUMS.minisig"),
                release_dir("v0.19.0", ASSET),
            ]
        );
        assert_eq!(std::fs::read(&dest).unwrap(), BINARY);
    }

    #[tokio::test]
    async fn a_signature_by_an_untrusted_key_never_reaches_the_binary() {
        let (signer, trusted) = (Signer::new(), Signer::new());
        let server = serve(release_files(&signer, "v0.19.0", "clawg v0.19.0 SHA256SUMS", BINARY)).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("candidate");

        let error = server
            .client()
            .fetch_binary(&release("v0.19.0"), TRIPLE, &[&trusted.public], &dest)
            .await
            .unwrap_err();

        assert!(matches!(error, FetchError::Signature(_)), "{error}");
        assert!(!server.requested().contains(&release_dir("v0.19.0", ASSET)));
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn hashes_signed_for_another_version_never_reach_the_binary() {
        let signer = Signer::new();
        let server = serve(release_files(&signer, "v0.19.0", "clawg v0.18.0 SHA256SUMS", BINARY)).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("candidate");

        let error = server
            .client()
            .fetch_binary(&release("v0.19.0"), TRIPLE, &[&signer.public], &dest)
            .await
            .unwrap_err();

        assert!(matches!(error, FetchError::Release(ReleaseError::SignedForAnotherVersion { .. })), "{error}");
        assert!(!server.requested().contains(&release_dir("v0.19.0", ASSET)));
    }

    #[tokio::test]
    async fn a_binary_that_does_not_match_its_hash_leaves_no_file() {
        let signer = Signer::new();
        let mut files = release_files(&signer, "v0.19.0", "clawg v0.19.0 SHA256SUMS", BINARY);
        files[2].1 = b"another executable".to_vec();
        let server = serve(files).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("candidate");

        let error = server
            .client()
            .fetch_binary(&release("v0.19.0"), TRIPLE, &[&signer.public], &dest)
            .await
            .unwrap_err();

        assert!(matches!(error, FetchError::HashMismatch { .. }), "{error}");
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn a_missing_file_names_its_address_and_status() {
        let server = Server::start(&[]).await;

        let error = server.client().list_releases().await.unwrap_err();

        let text = error.to_string();
        assert!(text.contains("/repos/MarcossIC/claw-garden/releases") && text.contains("404"), "{text}");
    }

    #[tokio::test]
    async fn a_body_over_its_cap_is_refused_while_it_is_read() {
        let server = Server::start(&[("/big", vec![b'x'; 1000])]).await;
        let url = format!("{}/big", server.base);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("candidate");

        assert!(matches!(server.client().get_capped(&url, 999).await, Err(FetchError::TooLarge { .. })));
        assert_eq!(server.client().get_capped(&url, 1000).await.unwrap().len(), 1000);
        assert!(matches!(
            server.client().download_hashed(&url, 999, &dest).await,
            Err(FetchError::TooLarge { .. })
        ));
        assert!(!dest.exists());
    }

    #[test]
    fn a_refusal_with_no_requests_left_is_the_rate_limit() {
        const NOW: u64 = 1_800_000_000;
        let reset = (NOW + 90).to_string();

        for status in [403, 429] {
            let error = refusal("https://api.github.com/x", status, Some("0"), Some(&reset), NOW);
            assert!(
                matches!(error, FetchError::RateLimited { resets_in_minutes: Some(2), .. }),
                "{status}: {error}"
            );
            assert!(error.to_string().contains("rate limit for this address is used up; try again in 2 min"), "{error}");
        }
        let unknown = refusal("https://api.github.com/x", 403, Some("0"), None, NOW).to_string();
        assert!(unknown.contains("try again within the hour"), "{unknown}");
        let past = refusal("https://api.github.com/x", 403, Some("0"), Some(&NOW.to_string()), NOW).to_string();
        assert!(past.contains("try again now"), "{past}");
    }

    #[test]
    fn a_refusal_with_requests_left_or_another_status_is_only_a_status() {
        for (status, remaining) in [(403, Some("12")), (403, None), (404, Some("0")), (500, Some("0"))] {
            let error = refusal("https://api.github.com/x", status, remaining, Some("1800000090"), 1_800_000_000);
            assert!(matches!(error, FetchError::Status { .. }), "{status} {remaining:?}: {error}");
        }
    }

    #[tokio::test]
    async fn the_rate_limit_is_read_from_the_answer() {
        let app = Router::new().fallback(|| async {
            (StatusCode::FORBIDDEN, [("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "4102444800")], "limited")
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let error = ReleaseClient::new(&base, &base).list_releases().await.unwrap_err();

        assert!(matches!(error, FetchError::RateLimited { status: 403, .. }), "{error}");
    }

    #[test]
    fn the_caps_are_the_documented_ones() {
        assert_eq!(LISTING_CAP, 1024 * 1024);
        assert_eq!(SUMS_CAP, 64 * 1024);
        assert_eq!(SIGNATURE_CAP, 4 * 1024);
        assert_eq!(BINARY_CAP, 256 * 1024 * 1024);
    }

    #[test]
    fn the_default_servers_are_githubs() {
        let client = ReleaseClient::github();
        assert_eq!(client.releases_url(), "https://api.github.com/repos/MarcossIC/claw-garden/releases?per_page=30");
        assert_eq!(
            client.asset_url("v0.19.0", "SHA256SUMS"),
            "https://github.com/MarcossIC/claw-garden/releases/download/v0.19.0/SHA256SUMS"
        );
    }
}
