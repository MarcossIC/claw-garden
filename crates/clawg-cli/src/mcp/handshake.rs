//! Tolerance for whatever a host sends before `initialize`.
//!
//! # The failure this exists for
//!
//! rmcp 1.3 accepts exactly two things as a session's first request:
//! `initialize`, and a `ping` before it. Anything else ends the server with
//! `ExpectedInitializeRequest` — the process exits without answering. Claude
//! Code 2.1.285 opens every stdio session with a `server/discover` probe for a
//! newer protocol lifecycle, and falls back to `initialize` only once the
//! probe comes back as an error. A server that dies instead of answering never
//! gets that far: the host waits out its whole connect timeout (30 s) and
//! reports `CONNECT_TIMEOUT`. Servers built on other SDKs (engram, notebooklm)
//! answer the probe with an error and connect in the legacy era in ~200 ms.
//!
//! # What the guard does
//!
//! [`PreInitGuard`] wraps the transport rmcp reads from. Until `initialize`
//! goes through it answers, on rmcp's behalf, every request rmcp would die on:
//! methods it has never heard of get `-32601 Method not found`, known methods
//! sent too early get `-32600 Invalid request`. Notifications and stray
//! responses have nobody to answer and are dropped. From `initialize` on the
//! guard is transparent — rmcp's own dispatcher already answers unknown
//! methods properly once a session exists.
//!
//! rmcp 3.x implements `server/discover` natively; this guard is the bridge's
//! answer for as long as it stays on 1.x.

use std::future::Future;

use rmcp::model::{
    ClientJsonRpcMessage, ClientRequest, ErrorCode, ErrorData, RequestId, ServerJsonRpcMessage,
};
use rmcp::service::RxJsonRpcMessage;
use rmcp::transport::Transport;
use rmcp::RoleServer;

/// What to do with one message that arrived before `initialize`.
#[derive(Debug, PartialEq)]
pub enum PreInit {
    /// `initialize` itself: hand it to rmcp; the session starts here.
    Initialize,
    /// Something rmcp already tolerates before `initialize` (`ping`).
    Forward,
    /// Answer request `id` with `error` on rmcp's behalf and keep waiting.
    Reject { id: RequestId, error: ErrorData },
    /// A notification or a stray response: nothing to answer, nothing rmcp
    /// can use before a session exists.
    Discard,
}

/// Classifies a message received before `initialize`. Pure, so every branch
/// is testable without a transport.
pub fn before_initialize(msg: &ClientJsonRpcMessage) -> PreInit {
    let ClientJsonRpcMessage::Request(req) = msg else {
        return PreInit::Discard;
    };
    let method = req.request.method();
    let error = match &req.request {
        ClientRequest::InitializeRequest(_) => return PreInit::Initialize,
        ClientRequest::PingRequest(_) => return PreInit::Forward,
        // rmcp parses every method it has no type for into `CustomRequest`,
        // so this arm is exactly "a method this server does not implement".
        ClientRequest::CustomRequest(_) => ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            format!("method not found: {method}"),
            None,
        ),
        _ => ErrorData::invalid_request(format!("{method} sent before initialize"), None),
    };
    PreInit::Reject {
        id: req.id.clone(),
        error,
    }
}

/// A transport that keeps rmcp alive through a host's pre-`initialize` probes.
/// See the module docs for why.
pub struct PreInitGuard<T> {
    inner: T,
    initialized: bool,
}

impl<T> PreInitGuard<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            initialized: false,
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
            if self.initialized {
                return Some(msg);
            }
            match before_initialize(&msg) {
                PreInit::Initialize => {
                    self.initialized = true;
                    return Some(msg);
                }
                PreInit::Forward => return Some(msg),
                PreInit::Reject { id, error } => {
                    log::debug!("answered a pre-initialize request: {}", error.message);
                    if let Err(e) = self.inner.send(ServerJsonRpcMessage::error(error, id)).await {
                        // The host is gone; there is no session left to start.
                        log::warn!("could not answer a pre-initialize request: {e}");
                        return None;
                    }
                }
                PreInit::Discard => log::debug!("dropped a pre-initialize message"),
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

    /// The probe exactly as Claude Code 2.1.285 opens a stdio session with it,
    /// minus the `_meta` payload, which rmcp moves into extensions anyway.
    const DISCOVER_PROBE: &str = r#"{"jsonrpc":"2.0","id":"server-discover-probe-1","method":"server/discover","params":{}}"#;

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

    /// The wire form of a rejection, so the tests assert what the host reads.
    fn rejection(action: PreInit) -> Value {
        let PreInit::Reject { id, error } = action else {
            panic!("expected a rejection, got {action:?}");
        };
        serde_json::to_value(ServerJsonRpcMessage::error(error, id)).expect("serialisable reply")
    }

    // ── before_initialize ───────────────────────────────────────────────────

    #[test]
    fn an_unknown_method_is_answered_as_method_not_found_under_its_own_id() {
        let reply = rejection(before_initialize(&parse(DISCOVER_PROBE)));
        assert_eq!(reply["id"], "server-discover-probe-1");
        assert_eq!(reply["error"]["code"], -32601);
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("server/discover"),
            "the message should name the method: {reply}"
        );
    }

    #[test]
    fn initialize_starts_the_session() {
        assert_eq!(before_initialize(&parse(&initialize(1))), PreInit::Initialize);
    }

    #[test]
    fn a_ping_is_left_to_rmcp_which_already_answers_it() {
        let ping = r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#;
        assert_eq!(before_initialize(&parse(ping)), PreInit::Forward);
    }

    #[test]
    fn a_known_method_sent_too_early_is_an_invalid_request_not_a_missing_one() {
        let early = r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#;
        let reply = rejection(before_initialize(&parse(early)));
        assert_eq!(reply["id"], 3);
        assert_eq!(reply["error"]["code"], -32600);
    }

    #[test]
    fn a_notification_before_initialize_is_dropped() {
        let cancelled = r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"server-discover-probe-1"}}"#;
        assert_eq!(before_initialize(&parse(cancelled)), PreInit::Discard);
    }

    #[test]
    fn a_stray_response_before_initialize_is_dropped() {
        let stray = r#"{"jsonrpc":"2.0","id":9,"result":{}}"#;
        assert_eq!(before_initialize(&parse(stray)), PreInit::Discard);
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
    async fn the_probe_is_answered_and_initialize_is_the_first_thing_rmcp_sees() {
        let init = initialize(1);
        let (inner, sent) = Scripted::new(&[DISCOVER_PROBE, &init]);
        let mut guard = PreInitGuard::new(inner);

        let first = guard.receive().await.expect("initialize reaches rmcp");
        assert_eq!(method_of(&first), "initialize");

        let sent = sent.lock().expect("sent lock");
        assert_eq!(sent.len(), 1, "exactly one reply, to the probe");
        let reply = serde_json::to_value(&sent[0]).expect("serialisable");
        assert_eq!(reply["id"], "server-discover-probe-1");
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn after_initialize_the_guard_forwards_everything_untouched() {
        let init = initialize(1);
        let later_unknown = r#"{"jsonrpc":"2.0","id":2,"method":"server/discover","params":{}}"#;
        let (inner, sent) = Scripted::new(&[&init, later_unknown]);
        let mut guard = PreInitGuard::new(inner);

        guard.receive().await.expect("initialize");
        let second = guard.receive().await.expect("forwarded, not answered");
        assert_eq!(method_of(&second), "server/discover");
        assert!(
            sent.lock().expect("sent lock").is_empty(),
            "once a session exists, answering is rmcp's job"
        );
    }

    #[tokio::test]
    async fn a_closed_stream_before_initialize_ends_the_session() {
        let (inner, _sent) = Scripted::new(&[DISCOVER_PROBE]);
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

    /// Drives `ClawgMcp` over newline-delimited JSON exactly as the stdio
    /// transport does, replaying the session opening that broke on
    /// 2026-09-29: probe first, then the legacy handshake, then a tool list.
    #[tokio::test]
    async fn a_discover_probe_no_longer_kills_the_bridge() {
        let (host, server) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server);
        let transport = PreInitGuard::new(AsyncRwTransport::new_server(server_read, server_write));

        // No daemon needed: the handshake and the tool list never call it.
        let bridge = ClawgMcp::new(
            DaemonClient::new("http://127.0.0.1:1", "token", std::env::temp_dir()),
            Profile::Dev,
        );
        tokio::spawn(async move {
            if let Ok(running) = bridge.serve(transport).await {
                let _ = running.waiting().await;
            }
        });

        let (host_read, mut host_write) = tokio::io::split(host);
        let mut lines = BufReader::new(host_read).lines();

        send_line(&mut host_write, DISCOVER_PROBE).await;
        let probe_reply = next_frame(&mut lines).await;
        assert_eq!(probe_reply["id"], "server-discover-probe-1");
        assert_eq!(probe_reply["error"]["code"], -32601);

        send_line(&mut host_write, &initialize(1)).await;
        let init_reply = next_frame(&mut lines).await;
        assert_eq!(init_reply["id"], 1);
        assert_eq!(init_reply["result"]["serverInfo"]["name"], "clawg-dev");

        send_line(&mut host_write, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).await;
        send_line(&mut host_write, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).await;
        let tools = next_frame(&mut lines).await;
        assert_eq!(tools["id"], 2);
        assert_eq!(
            tools["result"]["tools"].as_array().map(Vec::len),
            Some(7),
            "all seven memory tools are listed: {tools}"
        );
    }
}
