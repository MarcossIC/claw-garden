//! Tolerance for whatever a host sends before a session exists.
//!
//! # The failure this exists for
//!
//! Until a session starts, rmcp answers requests itself: `ping`,
//! `server/discover`, and any request missing the `_meta` the 2026-07-28
//! lifecycle needs. A session starts with `initialize` (the legacy lifecycle)
//! or with the first other request that carries that `_meta`. But a
//! notification or a stray response before then ends the server with
//! `ExpectedInitializeRequest`, and the process exits without a word. A host
//! that gives up on its `server/discover` probe and cancels it, as Claude Code
//! 2.1.285 can, sends exactly such a notification. A dead bridge leaves the
//! host waiting out its connect timeout (30 s) and reporting `CONNECT_TIMEOUT`.
//!
//! # What the guard does
//!
//! [`PreInitGuard`] wraps the transport rmcp reads from. Until a request that
//! starts a session goes through, it drops notifications and stray responses,
//! which have nobody to answer, and hands every request to rmcp. From then on
//! it is transparent.

use std::future::Future;

use rmcp::model::{ClientJsonRpcMessage, ClientRequest, ServerJsonRpcMessage};
use rmcp::service::RxJsonRpcMessage;
use rmcp::transport::Transport;
use rmcp::RoleServer;

/// What to do with one message that arrived before a session exists.
#[derive(Debug, PartialEq)]
pub enum PreInit {
    /// A request that can start a session: hand it to rmcp, which either
    /// starts one or answers it and keeps waiting.
    Start,
    /// A request rmcp answers without starting a session (`ping`,
    /// `server/discover`).
    Forward,
    /// A notification or a stray response: nothing to answer, and rmcp would
    /// end the server on it.
    Discard,
}

/// Classifies a message received before a session exists. Pure, so every
/// branch is testable without a transport.
pub fn before_session(msg: &ClientJsonRpcMessage) -> PreInit {
    let ClientJsonRpcMessage::Request(req) = msg else {
        return PreInit::Discard;
    };
    match &req.request {
        ClientRequest::PingRequest(_) | ClientRequest::DiscoverRequest(_) => PreInit::Forward,
        _ => PreInit::Start,
    }
}

/// A transport that keeps rmcp alive through what a host sends before a
/// session exists. See the module docs for why.
pub struct PreInitGuard<T> {
    inner: T,
    started: bool,
}

impl<T> PreInitGuard<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            started: false,
        }
    }
}

impl<T> Transport<RoleServer> for PreInitGuard<T>
where
    T: Transport<RoleServer>,
{
    type Error = T::Error;

    fn send(
        &mut self,
        item: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        loop {
            let msg = self.inner.receive().await?;
            if self.started {
                return Some(msg);
            }
            match before_session(&msg) {
                // Also a request rmcp rejects for missing `_meta`, which
                // starts nothing; the guard stands down anyway, because a
                // host that was just told its request is malformed retries
                // it before it sends anything else.
                PreInit::Start => {
                    self.started = true;
                    return Some(msg);
                }
                PreInit::Forward => return Some(msg),
                PreInit::Discard => log::debug!("dropped a message sent before a session"),
            }
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use clawg_core::paths::Profile;
    use rmcp::transport::async_rw::AsyncRwTransport;
    use rmcp::ServiceExt;
    use serde_json::{json, Value};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

    use crate::client::DaemonClient;
    use crate::mcp::server::ClawgMcp;

    /// The probe Claude Code 2.1.285 opens a stdio session with, without the
    /// `_meta` it carries.
    const BARE_PROBE: &str = r#"{"jsonrpc":"2.0","id":"server-discover-probe-1","method":"server/discover","params":{}}"#;

    const CANCEL_PROBE: &str = r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"server-discover-probe-1"}}"#;

    /// The `_meta` a 2026-07-28 client puts on every request.
    fn modern_meta() -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": { "name": "probe-host", "version": "0" }
        })
    }

    fn modern(id: u32, method: &str) -> String {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": { "_meta": modern_meta() } })
            .to_string()
    }

    fn parse(line: &str) -> ClientJsonRpcMessage {
        serde_json::from_str(line).expect("a valid client message")
    }

    fn initialize(id: u32) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "probe-host", "version": "0" }
            }
        })
        .to_string()
    }

    // ── before_session ──────────────────────────────────────────────────────

    #[test]
    fn a_discover_probe_is_left_to_rmcp_which_answers_it() {
        assert_eq!(before_session(&parse(BARE_PROBE)), PreInit::Forward);
    }

    #[test]
    fn a_ping_is_left_to_rmcp_which_answers_it() {
        let ping = r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#;
        assert_eq!(before_session(&parse(ping)), PreInit::Forward);
    }

    #[test]
    fn initialize_starts_the_session() {
        assert_eq!(before_session(&parse(&initialize(1))), PreInit::Start);
    }

    #[test]
    fn a_request_without_initialize_starts_the_session() {
        assert_eq!(before_session(&parse(&modern(3, "tools/list"))), PreInit::Start);
    }

    #[test]
    fn a_notification_before_a_session_is_dropped() {
        assert_eq!(before_session(&parse(CANCEL_PROBE)), PreInit::Discard);
    }

    #[test]
    fn a_stray_response_before_a_session_is_dropped() {
        let stray = r#"{"jsonrpc":"2.0","id":9,"result":{}}"#;
        assert_eq!(before_session(&parse(stray)), PreInit::Discard);
    }

    // ── PreInitGuard over a scripted transport ──────────────────────────────

    /// Replays a fixed script of incoming messages and records what is sent.
    struct Scripted {
        incoming: VecDeque<ClientJsonRpcMessage>,
        sent: Arc<Mutex<Vec<ServerJsonRpcMessage>>>,
    }

    impl Scripted {
        fn new(lines: &[&str]) -> (Self, Arc<Mutex<Vec<ServerJsonRpcMessage>>>) {
            let sent = Arc::new(Mutex::new(Vec::new()));
            let incoming = lines.iter().map(|l| parse(l)).collect();
            (
                Self {
                    incoming,
                    sent: sent.clone(),
                },
                sent,
            )
        }
    }

    impl Transport<RoleServer> for Scripted {
        type Error = std::io::Error;

        fn send(
            &mut self,
            item: ServerJsonRpcMessage,
        ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
            self.sent.lock().expect("sent lock").push(item);
            std::future::ready(Ok(()))
        }

        async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
            self.incoming.pop_front()
        }

        fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
            std::future::ready(Ok(()))
        }
    }

    fn method_of(msg: &ClientJsonRpcMessage) -> String {
        match msg {
            ClientJsonRpcMessage::Request(r) => r.request.method().to_string(),
            other => panic!("expected a request, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_probe_reaches_rmcp_and_its_cancellation_does_not() {
        let init = initialize(1);
        let (inner, sent) = Scripted::new(&[BARE_PROBE, CANCEL_PROBE, &init]);
        let mut guard = PreInitGuard::new(inner);

        let first = guard.receive().await.expect("the probe");
        assert_eq!(method_of(&first), "server/discover");
        let second = guard.receive().await.expect("initialize");
        assert_eq!(method_of(&second), "initialize");
        assert!(sent.lock().expect("sent lock").is_empty(), "answering is rmcp's job");
    }

    #[tokio::test]
    async fn after_the_session_starts_the_guard_forwards_everything_untouched() {
        let init = initialize(1);
        let initialized = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let (inner, _sent) = Scripted::new(&[&init, initialized]);
        let mut guard = PreInitGuard::new(inner);

        guard.receive().await.expect("initialize");
        let second = guard.receive().await.expect("forwarded, not dropped");
        assert!(matches!(second, ClientJsonRpcMessage::Notification(_)), "{second:?}");
    }

    #[tokio::test]
    async fn a_closed_stream_before_a_session_ends_it() {
        let (inner, _sent) = Scripted::new(&[CANCEL_PROBE]);
        let mut guard = PreInitGuard::new(inner);
        assert!(guard.receive().await.is_none());
    }

    // ── the real server, end to end over a byte stream ──────────────────────

    type HostLines = tokio::io::Lines<BufReader<ReadHalf<DuplexStream>>>;

    /// Writes one frame the way the stdio transport frames it: a JSON line.
    async fn send_line(host: &mut WriteHalf<DuplexStream>, line: &str) {
        host.write_all(format!("{line}\n").as_bytes())
            .await
            .expect("host write");
    }

    /// Reads the bridge's next frame. A dead bridge closes the stream, which
    /// is exactly the failure being guarded against, so it fails loudly.
    async fn next_frame(lines: &mut HostLines) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("the bridge answers within 5 s")
            .expect("readable stream")
            .expect("the bridge is still alive");
        serde_json::from_str(&line).expect("a JSON-RPC line")
    }

    /// Serves `ClawgMcp` over newline-delimited JSON exactly as the stdio
    /// transport does. No daemon: nothing these tests send reaches it.
    fn serve_bridge() -> (WriteHalf<DuplexStream>, HostLines) {
        let (host, server) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = PreInitGuard::new(AsyncRwTransport::new_server(server_read, server_write));
        let bridge = ClawgMcp::new(
            DaemonClient::new("http://127.0.0.1:1", "token", std::env::temp_dir()),
            Profile::Dev,
        );
        tokio::spawn(async move {
            if let Ok(running) = bridge.serve(transport).await {
                let _ = running.waiting().await;
            }
        });
        let (host_read, host_write) = tokio::io::split(host);
        (host_write, BufReader::new(host_read).lines())
    }

    fn tool_names(reply: &Value) -> Vec<String> {
        reply["result"]["tools"]
            .as_array()
            .map(|tools| tools.iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    /// The session opening that broke on 2026-09-29: probe first, then its
    /// cancellation, then the legacy handshake and a tool list.
    #[tokio::test]
    async fn a_bare_probe_and_its_cancellation_leave_the_legacy_handshake_working() {
        let (mut host, mut lines) = serve_bridge();

        send_line(&mut host, BARE_PROBE).await;
        let probe_reply = next_frame(&mut lines).await;
        assert_eq!(probe_reply["id"], "server-discover-probe-1");
        assert!(probe_reply["error"].is_object(), "a probe without _meta is refused: {probe_reply}");
        send_line(&mut host, CANCEL_PROBE).await;

        send_line(&mut host, &initialize(1)).await;
        let init_reply = next_frame(&mut lines).await;
        assert_eq!(init_reply["id"], 1);
        assert_eq!(init_reply["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init_reply["result"]["serverInfo"]["name"], "clawg-dev");

        send_line(&mut host, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).await;
        send_line(&mut host, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).await;
        let tools = next_frame(&mut lines).await;
        assert_eq!(tools["id"], 2);
        assert_eq!(tool_names(&tools).len(), 7, "all seven memory tools are listed: {tools}");
    }

    #[tokio::test]
    async fn a_discover_probe_is_answered_with_what_the_server_offers() {
        let (mut host, mut lines) = serve_bridge();

        send_line(&mut host, &modern(1, "server/discover")).await;
        let reply = next_frame(&mut lines).await;

        let result = &reply["result"];
        let versions: Vec<&str> = result["supportedVersions"]
            .as_array()
            .map(|v| v.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        assert!(versions.contains(&"2026-07-28"), "{reply}");
        assert!(versions.contains(&"2025-06-18"), "legacy hosts keep their version: {reply}");
        assert!(result["capabilities"]["tools"].is_object(), "{reply}");
        assert!(
            result["instructions"].as_str().is_some_and(|i| i.contains("save_memory")),
            "{reply}"
        );
        let server = &result["_meta"]["io.modelcontextprotocol/serverInfo"];
        assert_eq!(server["name"], "clawg-dev");
        assert_eq!(server["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn a_session_without_initialize_lists_the_tools_in_name_order() {
        let (mut host, mut lines) = serve_bridge();

        send_line(&mut host, &modern(1, "server/discover")).await;
        next_frame(&mut lines).await;
        send_line(&mut host, &modern(2, "tools/list")).await;
        let reply = next_frame(&mut lines).await;

        let names = tool_names(&reply);
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names.len(), 7, "{reply}");
        assert_eq!(names, sorted, "the order must not change between calls");
        let result = &reply["result"];
        assert_eq!(result["resultType"], "complete", "{reply}");
        assert!(result["ttlMs"].is_u64(), "{reply}");
        assert!(result["cacheScope"].is_string(), "{reply}");
    }
}
