//! Masking of secrets with a known format: API keys, tokens, private keys and
//! passwords inside URLs; of literal values after a password, secret, token or
//! API key name or an `Authorization` header; and of whatever the writer marks
//! `<private>…</private>`.
//!
//! Only formats with a fixed prefix or structure, or a fixed key in front,
//! are recognised. Nothing here guesses from entropy: commit hashes, UUIDs and
//! checksums look random too, and they are memory worth keeping.

use std::ops::Range;

use once_cell::sync::Lazy;
use regex::Regex;

/// What kind of secret a match is. The name is what the placeholder carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretKind {
    AwsAccessKey,
    GithubToken,
    AnthropicKey,
    OpenaiKey,
    SlackToken,
    GoogleApiKey,
    StripeKey,
    GitlabToken,
    NpmToken,
    GoogleOauthSecret,
    SendgridKey,
    HuggingfaceToken,
    SlackWebhook,
    PrivateKey,
    Jwt,
    UrlPassword,
    Password,
    AuthHeader,
    Credential,
    Private,
}

impl SecretKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SecretKind::AwsAccessKey => "aws_access_key",
            SecretKind::GithubToken => "github_token",
            SecretKind::AnthropicKey => "anthropic_key",
            SecretKind::OpenaiKey => "openai_key",
            SecretKind::SlackToken => "slack_token",
            SecretKind::GoogleApiKey => "google_api_key",
            SecretKind::StripeKey => "stripe_key",
            SecretKind::GitlabToken => "gitlab_token",
            SecretKind::NpmToken => "npm_token",
            SecretKind::GoogleOauthSecret => "google_oauth_secret",
            SecretKind::SendgridKey => "sendgrid_key",
            SecretKind::HuggingfaceToken => "huggingface_token",
            SecretKind::SlackWebhook => "slack_webhook",
            SecretKind::PrivateKey => "private_key",
            SecretKind::Jwt => "jwt",
            SecretKind::UrlPassword => "url_password",
            SecretKind::Password => "password",
            SecretKind::AuthHeader => "auth_header",
            SecretKind::Credential => "credential",
            SecretKind::Private => "private",
        }
    }
}

/// One secret in a text: its byte range and kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub range: Range<usize>,
    pub kind: SecretKind,
}

/// A text with its secrets replaced by `[REDACTED:<kind>]`, and the kinds that
/// were replaced, each once, in order of first appearance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    pub text: String,
    pub kinds: Vec<SecretKind>,
}

/// One regex per kind, in the order a tie between two matches starting at the
/// same byte is settled. The capture group, when there is one, is what
/// [`accepts`] checks; in a URL and after a key it is also the only part
/// masked.
static PATTERNS: Lazy<Vec<(SecretKind, Regex)>> = Lazy::new(|| {
    [
        (SecretKind::AwsAccessKey, r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
        (SecretKind::GithubToken, r"\b(?:gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{22,})"),
        (SecretKind::AnthropicKey, r"\bsk-ant-[A-Za-z0-9_-]{20,}"),
        (SecretKind::OpenaiKey, r"\bsk-(?:proj-|svcacct-|admin-)?([A-Za-z0-9_-]{20,})"),
        (SecretKind::SlackToken, r"\b(?:xox[abposr]|xapp)-[A-Za-z0-9-]{10,}"),
        (SecretKind::GoogleApiKey, r"\bAIza[A-Za-z0-9_-]{35}"),
        (SecretKind::StripeKey, r"\b(?:[sr]k_live_[A-Za-z0-9]{16,}|whsec_[A-Za-z0-9]{24,})"),
        (SecretKind::GitlabToken, r"\bglpat-[A-Za-z0-9_-]{20,}"),
        (SecretKind::NpmToken, r"\bnpm_[A-Za-z0-9]{36}\b"),
        (SecretKind::GoogleOauthSecret, r"\bGOCSPX-[A-Za-z0-9_-]{28}"),
        (SecretKind::SendgridKey, r"\bSG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}"),
        (SecretKind::HuggingfaceToken, r"\bhf_([A-Za-z0-9]{34})\b"),
        (
            SecretKind::SlackWebhook,
            r"\bhttps://hooks\.slack\.com/services/T[A-Z0-9]{6,}/B[A-Z0-9]{6,}/[A-Za-z0-9]{20,}",
        ),
        (
            SecretKind::PrivateKey,
            r"-----BEGIN [A-Z ]*PRIVATE KEY(?: BLOCK)?-----(?s:.*?-----END [A-Z ]*PRIVATE KEY(?: BLOCK)?-----|.*)",
        ),
        (SecretKind::Jwt, r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]*"),
        (SecretKind::UrlPassword, r"\b[A-Za-z][A-Za-z0-9+.-]*://[^\s:/@]+:([^\s/@]+)@"),
        // A value never starts with `:`: `password::x` is a path through a
        // module, not a key and its value.
        (
            SecretKind::Password,
            r#"(?i)\b\w*pass(?:word|wd)["']?\s*(?:==|:=|=|:)\s*["']?([^\s"'`,;)}\]:][^\s"'`,;)}\]]*)"#,
        ),
        (
            SecretKind::AuthHeader,
            r#"(?i)\bauthorization["']?\s*[:=]\s*["']?(?:bearer|basic|token)\s+([^\s"'`,;)}\]]+)"#,
        ),
        (
            SecretKind::Credential,
            r#"(?i)\b[a-z0-9_-]*?(?:secret|token|api[_-]?key|access[_-]?key|secret[_-]?key|account[_-]?key|auth[_-]?key)["']?\s*(?:==|:=|=|:)\s*["']?([^\s"'`,;)}\]:][^\s"'`,;)}\]]*)"#,
        ),
    ]
    .into_iter()
    .map(|(kind, pattern)| (kind, Regex::new(pattern).expect("secret patterns are valid")))
    .collect()
});

static PRIVATE_OPEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)<private>").expect("the mark is valid"));
static PRIVATE_CLOSE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)</private>").expect("the mark is valid"));

/// The spans the writer marked private, marks included. A mark left open
/// runs to the end: whoever opened it said what follows is secret. A mark
/// right after a backtick is quoted, so text about the mark loses nothing.
fn private_spans(text: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(open) = PRIVATE_OPEN.find_at(text, at) {
        if text[..open.start()].ends_with('`') {
            at = open.end();
            continue;
        }
        let end = PRIVATE_CLOSE.find_at(text, open.end()).map_or(text.len(), |close| close.end());
        spans.push(open.start()..end);
        at = end;
    }
    spans
}

/// Whether a match the regex alone accepts is really one. `secret` is the
/// part that would be masked.
fn accepts(kind: SecretKind, secret: &str) -> bool {
    match kind {
        // `sk-` alone is a common prefix in identifiers, paths and package
        // names, which are almost never mixed case. A real key is random
        // base62 or base64url, so it has upper and lower case letters and
        // digits; `sk-ant-` is Anthropic's.
        SecretKind::OpenaiKey => !secret.starts_with("ant-") && mixes_case_and_digits(secret),
        // 34 letters after `hf_` can be a class name
        // (`AutoModelForSequenceClassification`); a token is random base62.
        SecretKind::HuggingfaceToken => mixes_case_and_digits(secret),
        // A variable, a template slot, an earlier placeholder or a masked
        // value: none of them is the password.
        SecretKind::UrlPassword => !secret.starts_with(['$', '<', '{', '[', '*']),
        SecretKind::Password | SecretKind::AuthHeader => is_literal_secret(secret),
        // After these keys a number is a limit or a count (`max_token=100000`)
        // and `a::b` a path, far more often than a secret.
        SecretKind::Credential => {
            is_literal_secret(secret) && secret.chars().any(|c| c.is_ascii_alphabetic()) && !secret.contains("::")
        }
        _ => true,
    }
}

fn mixes_case_and_digits(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_digit())
        && s.bytes().any(|b| b.is_ascii_uppercase())
        && s.bytes().any(|b| b.is_ascii_lowercase())
}

/// Whether the value after a password, secret, token or API key name or an
/// `Authorization` scheme is a literal secret rather than code, a type, a
/// placeholder or prose. A word with no digit and no symbol is let through:
/// it is far more often a type or a variable than a password, and masking it
/// destroys the memory.
fn is_literal_secret(value: &str) -> bool {
    let is_identifier = |s: &str| {
        let mut chars = s.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    let is_dotted_path = value.contains('.') && value.split('.').all(is_identifier);
    value.chars().count() >= 6
        && !value.starts_with(['$', '%', '*'])
        && !value.contains(['(', ')', '[', ']', '{', '}', '<', '>'])
        && !is_dotted_path
        && value.chars().any(|c| c.is_ascii_digit() || !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
}

/// Every secret in `text`, in order, none overlapping. Of two that overlap,
/// the one that starts first wins, and of two that start together, the
/// longer.
pub fn find_secrets(text: &str) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    for (kind, regex) in PATTERNS.iter() {
        for caps in regex.captures_iter(text) {
            let whole = caps.get(0).expect("group 0 is the whole match");
            let part = caps.get(1).unwrap_or(whole);
            if !accepts(*kind, part.as_str()) {
                continue;
            }
            let value_only = matches!(
                kind,
                SecretKind::UrlPassword | SecretKind::Password | SecretKind::AuthHeader | SecretKind::Credential
            );
            let masked = if value_only { part } else { whole };
            found.push(Found { range: masked.range(), kind: *kind });
        }
    }
    found.extend(private_spans(text).into_iter().map(|range| Found { range, kind: SecretKind::Private }));
    // Stable, so a tie in start and length keeps the pattern order.
    found.sort_by(|a, b| a.range.start.cmp(&b.range.start).then(b.range.end.cmp(&a.range.end)));
    let mut kept: Vec<Found> = Vec::with_capacity(found.len());
    for f in found {
        if kept.last().map_or(true, |last| f.range.start >= last.range.end) {
            kept.push(f);
        }
    }
    kept
}

/// `text` with every secret [`find_secrets`] reports replaced by its
/// placeholder.
pub fn redact(text: &str) -> Redacted {
    let mut out = String::with_capacity(text.len());
    let mut kinds: Vec<SecretKind> = Vec::new();
    let mut at = 0;
    for Found { range, kind } in find_secrets(text) {
        out.push_str(&text[at..range.start]);
        out.push_str("[REDACTED:");
        out.push_str(kind.as_str());
        out.push(']');
        at = range.end;
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    out.push_str(&text[at..]);
    Redacted { text: out, kinds }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built at run time so no literal in the repository looks like a live
    // credential to a secret scanner.
    fn repeat(unit: &str, len: usize) -> String {
        unit.chars().cycle().take(len).collect()
    }

    fn one_of_each() -> Vec<(String, SecretKind)> {
        vec![
            (format!("AKIA{}", repeat("Q7ZX", 16)), SecretKind::AwsAccessKey),
            (format!("ghp_{}", repeat("a1B2", 36)), SecretKind::GithubToken),
            (format!("sk-ant-api03-{}", repeat("x9Y_", 40)), SecretKind::AnthropicKey),
            (format!("sk-proj-{}", repeat("k3Lm", 48)), SecretKind::OpenaiKey),
            (format!("xoxb-{}", repeat("12345-", 24)), SecretKind::SlackToken),
            (format!("AIza{}", repeat("Sy9-", 35)), SecretKind::GoogleApiKey),
            (format!("sk_live_{}", repeat("4eC3", 24)), SecretKind::StripeKey),
            (format!("glpat-{}", repeat("x7Y-9z", 20)), SecretKind::GitlabToken),
            (format!("npm_{}", repeat("Ab3d", 36)), SecretKind::NpmToken),
            (format!("GOCSPX-{}", repeat("q2W_e4", 28)), SecretKind::GoogleOauthSecret),
            (format!("SG.{}.{}", repeat("k9-Lm", 22), repeat("Zx8_y", 43)), SecretKind::SendgridKey),
            (
                format!(
                    "-----BEGIN RSA PRIVATE KEY-----\n{}\n-----END RSA PRIVATE KEY-----",
                    repeat("MIIEow", 64)
                ),
                SecretKind::PrivateKey,
            ),
            (
                format!("eyJ{}.eyJ{}.{}", repeat("hbGc", 20), repeat("zdWI", 30), repeat("SflK", 43)),
                SecretKind::Jwt,
            ),
            (format!("hf_{}", repeat("aB3d", 34)), SecretKind::HuggingfaceToken),
            (
                format!(
                    "https://hooks.slack.com/services/T{}/B{}/{}",
                    repeat("Q7ZX", 9),
                    repeat("K2LM", 9),
                    repeat("x9Yz", 24)
                ),
                SecretKind::SlackWebhook,
            ),
        ]
    }

    #[test]
    fn every_known_secret_format_is_masked_in_place() {
        let mut text = String::from("notes:");
        let mut expected = String::from("notes:");
        let mut kinds = Vec::new();
        for (secret, kind) in one_of_each() {
            text.push_str(&format!(" use {secret} here,"));
            expected.push_str(&format!(" use [REDACTED:{}] here,", kind.as_str()));
            kinds.push(kind);
        }
        text.push_str(" and postgres://app:s3cr3t-pass@db:5432/app.");
        expected.push_str(" and postgres://app:[REDACTED:url_password]@db:5432/app.");
        kinds.push(SecretKind::UrlPassword);

        let out = redact(&text);
        assert_eq!(out.text, expected);
        assert_eq!(out.kinds, kinds);
    }

    #[test]
    fn ordinary_identifiers_and_hashes_are_left_alone() {
        let text = "commit 3f786850e387550fdab836ed7e6dc881de23001b, \
                    uuid 123e4567-e89b-12d3-a456-426614174000, \
                    sha256 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08, \
                    MAX_LEXICAL_CANDIDATES_PER_QUERY_WINDOW, scikit-learn, \
                    sk-learn-is-not-a-key-at-all, AKIA alone, ghp_short, \
                    postgres://app:${DB_PASSWORD}@db:5432/app, \
                    https://user:<password>@example.com, https://example.com/a@b, \
                    risk-assessment-for-the-quarterly-plan-2026, hf_hub_download, \
                    https://hooks.slack.com/services/your-webhook-here";
        // Joined at run time: a secret scanner takes the literal for a token.
        let text = format!("{text}, hf_{}", "AutoModelForSequenceClassification");
        let out = redact(&text);
        assert_eq!(out.text, text);
        assert!(out.kinds.is_empty(), "{:?}", out.kinds);
    }

    #[test]
    fn a_hyphenated_identifier_after_sk_is_left_alone() {
        let text = "the sk-learn-pipeline-for-text-classification-v2-2026-model and \
                    scripts/sk-build-2026-release-candidate-01";
        let out = redact(text);
        assert_eq!(out.text, text);
        assert!(out.kinds.is_empty(), "{:?}", out.kinds);
    }

    #[test]
    fn every_form_of_an_openai_key_is_masked() {
        for key in [
            format!("sk-{}", repeat("T3Bl9bkFJ", 48)),
            format!("sk-proj-{}", repeat("aZ0_-9xQ", 156)),
            format!("sk-svcacct-{}", repeat("Qw8-eR7_", 120)),
            format!("sk-admin-{}", repeat("m4N_b5V-", 120)),
        ] {
            let out = redact(&format!("key {key} end"));
            assert_eq!(out.text, "key [REDACTED:openai_key] end", "{key}");
        }
    }

    #[test]
    fn a_private_span_is_masked_with_its_marks() {
        let out = redact("user is <private>Ana, 34</private> and <PRIVATE>vive en Rosario</Private>.");
        assert_eq!(out.text, "user is [REDACTED:private] and [REDACTED:private].");
        assert_eq!(out.kinds, vec![SecretKind::Private]);
    }

    #[test]
    fn an_open_private_mark_masks_to_the_end() {
        let out = redact("keep this <private>card 4111 and everything after\nand the next line");
        assert_eq!(out.text, "keep this [REDACTED:private]");
        assert_eq!(out.kinds, vec![SecretKind::Private]);
    }

    #[test]
    fn a_mark_quoted_in_backticks_is_text() {
        let text = "wrap secrets in `<private>…</private>`; an open `<private>` masks to the end";
        let out = redact(text);
        assert_eq!(out.text, text);
        assert!(out.kinds.is_empty(), "{:?}", out.kinds);

        let out = redact("the mark `<private>` hides <private>this</private> only");
        assert_eq!(out.text, "the mark `<private>` hides [REDACTED:private] only");
    }

    #[test]
    fn a_private_span_covers_the_secrets_inside_it() {
        let token = format!("ghp_{}", repeat("a1B2", 36));
        let out = redact(&format!("<private>token {token}</private> rest"));
        assert_eq!(out.text, "[REDACTED:private] rest");
        assert_eq!(out.kinds, vec![SecretKind::Private]);
    }

    #[test]
    fn a_literal_password_is_masked_and_its_key_kept() {
        for (text, expected) in [
            ("password=hunter2", "password=[REDACTED:password]"),
            ("export DB_PASSWORD=\"s3cr3t!x\"", "export DB_PASSWORD=\"[REDACTED:password]\""),
            ("{\"password\": \"p@ssw0rd\"}", "{\"password\": \"[REDACTED:password]\"}"),
            ("PGPASSWORD=Zx9-kq2 psql", "PGPASSWORD=[REDACTED:password] psql"),
            ("db_passwd: 'open-sesame-77'", "db_passwd: '[REDACTED:password]'"),
            ("pass := \"x\"; password := \"tr0ub4dor\"", "pass := \"x\"; password := \"[REDACTED:password]\""),
            ("password==s3cr3t9", "password==[REDACTED:password]"),
        ] {
            let out = redact(text);
            assert_eq!(out.text, expected, "{text}");
            assert_eq!(out.kinds, vec![SecretKind::Password], "{text}");
        }
    }

    #[test]
    fn code_and_placeholders_after_a_password_key_are_left_alone() {
        for text in [
            "password=<tu-clave>",
            "password: string",
            "password: Option<String>,",
            "password: Optional[str] = None",
            "password = os.getenv(\"DB\")",
            "password: cfg.db.password",
            "PGPASSWORD=$PGPASS",
            "password=${DB_PASSWORD}",
            "password: change-me",
            "password: YOUR_PASSWORD_HERE",
            "password: ******",
            "password=%DB_PASS%",
            "password_hash: \"$2b$12$abcdefghijk\"",
            "passwordFile=/etc/app/pass1",
            "PWD=/home/user/app2",
            "WHERE password = ?",
            "password=[REDACTED:url_password]",
            "use crate::auth::password::rules::check;",
            "use crate::password::hash_v2;",
            "algo: Password::Argon2id",
        ] {
            let out = redact(text);
            assert_eq!(out.text, text);
            assert!(out.kinds.is_empty(), "{text}: {:?}", out.kinds);
        }
    }

    #[test]
    fn a_literal_credential_is_masked_and_its_key_kept() {
        let aws = repeat("wJalrXUtnFEMI/K7MDENG+bPxRfiCY", 40);
        let router = format!("sk-or-v1-{}", repeat("0f3a9c", 64));
        let client = repeat("Zq8-xW2_", 34);
        let api = repeat("d41d8cd98f00b204", 32);
        let account = format!("{}==", repeat("Eby8vdM02xNOcqFl", 86));
        for (text, expected) in [
            (format!("AWS_SECRET_ACCESS_KEY={aws}"), "AWS_SECRET_ACCESS_KEY=[REDACTED:credential]".to_string()),
            (format!("OPENROUTER_API_KEY={router}"), "OPENROUTER_API_KEY=[REDACTED:credential]".to_string()),
            (
                format!("{{\"client_secret\": \"{client}\"}}"),
                "{\"client_secret\": \"[REDACTED:credential]\"}".to_string(),
            ),
            (format!("curl -H 'X-Api-Key: {api}'"), "curl -H 'X-Api-Key: [REDACTED:credential]'".to_string()),
            (
                format!("DefaultEndpointsProtocol=https;AccountName=app;AccountKey={account};"),
                "DefaultEndpointsProtocol=https;AccountName=app;AccountKey=[REDACTED:credential];".to_string(),
            ),
            ("GITHUB_TOKEN: a1b2c3d4e5f6".to_string(), "GITHUB_TOKEN: [REDACTED:credential]".to_string()),
            ("secret = \"zebra-quokka-7741\"".to_string(), "secret = \"[REDACTED:credential]\"".to_string()),
            ("githubToken := \"q9W-e8R7t\"".to_string(), "githubToken := \"[REDACTED:credential]\"".to_string()),
        ] {
            let out = redact(&text);
            assert_eq!(out.text, expected, "{text}");
            assert_eq!(out.kinds, vec![SecretKind::Credential], "{text}");
        }
    }

    #[test]
    fn code_and_placeholders_after_a_credential_key_are_left_alone() {
        for text in [
            "token = next_token()",
            "secret: string",
            "api_key: Option<String>,",
            "let token = parser.token;",
            "max_token=100000",
            "max_tokens: 4096",
            "GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}",
            "api_key=$OPENAI_API_KEY",
            "secret_name: my-app",
            "tokenizer: bert-base-2",
            "api_key: YOUR_API_KEY",
            "access_key: <your-access-key>",
            "client_secret = os.environ[\"CLIENT_SECRET\"]",
            "token: Token::Ident",
            "the secret: keep it out of the repo",
            "csrf_token: true",
            "id_token: [REDACTED:jwt]",
        ] {
            let out = redact(text);
            assert_eq!(out.text, text);
            assert!(out.kinds.is_empty(), "{text}: {:?}", out.kinds);
        }
    }

    #[test]
    fn an_authorization_header_masks_only_its_credential() {
        let out = redact("curl -H \"Authorization: Bearer a1b2c3d4e5f6g7h8\" https://api.example.com");
        assert_eq!(out.text, "curl -H \"Authorization: Bearer [REDACTED:auth_header]\" https://api.example.com");
        assert_eq!(out.kinds, vec![SecretKind::AuthHeader]);

        let out = redact("authorization: basic dXNlcjpwYXNzd29yZA==");
        assert_eq!(out.text, "authorization: basic [REDACTED:auth_header]");

        for text in [
            "Authorization: Bearer $TOKEN",
            "Authorization: Bearer <token>",
            "Authorization: Bearer YOUR_TOKEN",
            "headers = {\"Authorization\": f\"Bearer {token}\"}",
            "send a Bearer token in the Authorization header",
        ] {
            let out = redact(text);
            assert_eq!(out.text, text);
            assert!(out.kinds.is_empty(), "{text}: {:?}", out.kinds);
        }
    }

    #[test]
    fn a_known_format_wins_over_its_context() {
        let token = format!("ghp_{}", repeat("a1B2", 36));
        let out = redact(&format!("Authorization: Bearer {token}"));
        assert_eq!(out.text, "Authorization: Bearer [REDACTED:github_token]");
        assert_eq!(out.kinds, vec![SecretKind::GithubToken]);

        let key = format!("sk-proj-{}", repeat("k3Lm", 48));
        let out = redact(&format!("OPENAI_PASSWORD={key}"));
        assert_eq!(out.text, "OPENAI_PASSWORD=[REDACTED:openai_key]");
        assert_eq!(out.kinds, vec![SecretKind::OpenaiKey]);

        let token = format!("hf_{}", repeat("aB3d", 34));
        let out = redact(&format!("HF_TOKEN={token}"));
        assert_eq!(out.text, "HF_TOKEN=[REDACTED:huggingface_token]");
        assert_eq!(out.kinds, vec![SecretKind::HuggingfaceToken]);

        let key = format!("AKIA{}", repeat("Q7ZX", 16));
        let out = redact(&format!("aws_access_key: {key}"));
        assert_eq!(out.text, "aws_access_key: [REDACTED:aws_access_key]");
    }

    #[test]
    fn a_url_keeps_everything_but_its_password() {
        let out = redact("DATABASE_URL=postgres://app:s3cr3t-pass@db.internal:5432/app");
        assert_eq!(out.text, "DATABASE_URL=postgres://app:[REDACTED:url_password]@db.internal:5432/app");
        assert_eq!(out.kinds, vec![SecretKind::UrlPassword]);
    }

    #[test]
    fn a_private_key_without_its_end_line_is_masked_to_the_end() {
        let text = format!("config:\n-----BEGIN OPENSSH PRIVATE KEY-----\n{}", repeat("b3Bl", 70));
        let out = redact(&text);
        assert_eq!(out.text, "config:\n[REDACTED:private_key]");
        assert_eq!(out.kinds, vec![SecretKind::PrivateKey]);
    }

    #[test]
    fn a_pgp_private_key_block_is_masked() {
        let block = format!(
            "-----BEGIN PGP PRIVATE KEY BLOCK-----\n\n{}\n-----END PGP PRIVATE KEY BLOCK-----",
            repeat("lQdGBF", 64)
        );
        let out = redact(&format!("signing key:\n{block}\nkeep this"));
        assert_eq!(out.text, "signing key:\n[REDACTED:private_key]\nkeep this");
        assert_eq!(out.kinds, vec![SecretKind::PrivateKey]);
    }

    #[test]
    fn the_slack_app_and_stripe_webhook_forms_are_masked() {
        for (secret, kind) in [
            (format!("xapp-1-{}", repeat("A0b1-", 40)), SecretKind::SlackToken),
            (format!("whsec_{}", repeat("Mk7p", 32)), SecretKind::StripeKey),
        ] {
            let out = redact(&format!("use {secret} here"));
            assert_eq!(out.text, format!("use [REDACTED:{}] here", kind.as_str()), "{secret}");
        }
    }

    #[test]
    fn masking_twice_finds_nothing_new() {
        let mut text = String::new();
        for (secret, _) in one_of_each() {
            text.push_str(&secret);
            text.push('\n');
        }
        text.push_str("mysql://root:hunter2@localhost/db\n");
        text.push_str("DB_PASSWORD=s3cr3t!x\nAuthorization: Bearer a1b2c3d4e5f6g7h8\nAPI_KEY=q9W-e8R7t\n");
        text.push_str("<private>the vault code</private> and <private>an open mark\n");
        let once = redact(&text);
        for kind in [
            SecretKind::UrlPassword,
            SecretKind::Password,
            SecretKind::AuthHeader,
            SecretKind::Credential,
            SecretKind::Private,
        ] {
            assert!(once.kinds.contains(&kind), "{kind:?} in {:?}", once.kinds);
        }
        let twice = redact(&once.text);
        assert_eq!(twice.text, once.text);
        assert!(twice.kinds.is_empty(), "{:?}", twice.kinds);
    }

    #[test]
    fn an_anthropic_key_is_never_reported_as_an_openai_one() {
        let key = format!("sk-ant-api03-{}", repeat("a1B2", 90));
        let found = find_secrets(&format!("key {key} end"));
        assert_eq!(found, vec![Found { range: 4..4 + key.len(), kind: SecretKind::AnthropicKey }]);
    }

    #[test]
    fn a_kind_found_twice_is_reported_once() {
        let a = format!("ghp_{}", repeat("a1B2", 36));
        let b = format!("ghp_{}", repeat("c3D4", 36));
        let out = redact(&format!("{a} and {b}"));
        assert_eq!(out.text, "[REDACTED:github_token] and [REDACTED:github_token]");
        assert_eq!(out.kinds, vec![SecretKind::GithubToken]);
    }
}
