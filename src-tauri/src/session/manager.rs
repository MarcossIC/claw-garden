use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use clawg_core::models::chat::{current_time_millis, generate_message_id, ChatMessage, Role};
use clawg_core::models::session::{PersistedMessage, SessionMeta, SessionRecord};

use super::SessionError;

pub struct SessionManager {
    base_dir: PathBuf,
    current_session_id: String,
    meta: SessionMeta,
}

impl SessionManager {
    /// Create a new session.
    pub fn new_session(base_dir: &Path) -> Result<Self, SessionError> {
        fs::create_dir_all(base_dir)?;

        let session_id = generate_session_id();
        let session_dir = base_dir.join(&session_id);
        fs::create_dir_all(&session_dir)?;

        let now = current_time_millis();
        let meta = SessionMeta {
            version: 1,
            session_id: session_id.clone(),
            created_at_ms: now,
            updated_at_ms: now,
            agents: vec![],
        };

        let meta_path = session_dir.join("meta.json");
        let meta_json = serde_json::to_string_pretty(&meta)?;
        fs::write(&meta_path, &meta_json)?;

        Ok(Self {
            base_dir: base_dir.to_path_buf(),
            current_session_id: session_id,
            meta,
        })
    }

    /// Resume an existing session.
    pub fn load_session(base_dir: &Path, session_id: &str) -> Result<Self, SessionError> {
        let session_dir = base_dir.join(session_id);
        let meta_path = session_dir.join("meta.json");

        if !meta_path.exists() {
            return Err(SessionError::NotFound(session_id.to_string()));
        }

        let meta_str = fs::read_to_string(&meta_path)?;
        let meta: SessionMeta = serde_json::from_str(&meta_str)?;

        Ok(Self {
            base_dir: base_dir.to_path_buf(),
            current_session_id: session_id.to_string(),
            meta,
        })
    }

    /// Load the latest session (by updated_at_ms).
    pub fn load_latest(base_dir: &Path) -> Result<Option<Self>, SessionError> {
        if !base_dir.exists() {
            return Ok(None);
        }

        let mut sessions: Vec<(u64, String)> = vec![];

        for entry in fs::read_dir(base_dir)?.flatten() {
            let meta_path = entry.path().join("meta.json");
            if meta_path.exists() {
                if let Ok(content) = fs::read_to_string(&meta_path) {
                    if let Ok(meta) = serde_json::from_str::<SessionMeta>(&content) {
                        sessions.push((meta.updated_at_ms, meta.session_id));
                    }
                }
            }
        }

        sessions.sort_by(|a, b| b.0.cmp(&a.0));

        match sessions.first() {
            Some((_, id)) => Self::load_session(base_dir, id).map(Some),
            None => Ok(None),
        }
    }

    /// Append a message for an agent (append-only, fast).
    pub fn append_message(
        &mut self,
        agent_id: &str,
        message: &ChatMessage,
    ) -> Result<(), SessionError> {
        if !self.meta.agents.contains(&agent_id.to_string()) {
            self.meta.agents.push(agent_id.to_string());
        }

        let record = SessionRecord::Message {
            message: PersistedMessage {
                id: message.id.clone(),
                role: message.role.clone(),
                content: message.content.clone(),
                tool_calls: message.tool_calls.clone(),
                timestamp_ms: message.timestamp_ms,
            },
        };

        let jsonl_path = self.agent_file_path(agent_id);
        let line = serde_json::to_string(&record)?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&jsonl_path)?;
        writeln!(file, "{}", line)?;
        file.flush()?;

        // Single meta write (covers both agent addition and timestamp update)
        self.meta.updated_at_ms = current_time_millis();
        self.save_meta()?;

        Ok(())
    }

    /// Load all messages for an agent.
    pub fn load_agent_messages(
        &self,
        agent_id: &str,
    ) -> Result<Vec<ChatMessage>, SessionError> {
        let jsonl_path = self.agent_file_path(agent_id);

        if !jsonl_path.exists() {
            return Ok(vec![]);
        }

        let contents = fs::read_to_string(&jsonl_path)?;
        let mut messages = vec![];

        for (line_number, raw_line) in contents.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<SessionRecord>(line) {
                Ok(SessionRecord::Message { message }) => {
                    messages.push(ChatMessage {
                        id: message.id,
                        role: message.role,
                        content: message.content,
                        tool_calls: message.tool_calls,
                        timestamp_ms: message.timestamp_ms,
                        is_error: None,
                    });
                }
                Ok(SessionRecord::Compaction { summary, .. }) => {
                    messages.clear();
                    messages.push(ChatMessage {
                        id: generate_message_id(),
                        role: Role::System,
                        content: summary,
                        tool_calls: None,
                        timestamp_ms: current_time_millis(),
                        is_error: None,
                    });
                }
                Err(e) => {
                    log::warn!(
                        "Skipping malformed JSONL line {} in {}: {}",
                        line_number + 1,
                        jsonl_path.display(),
                        e
                    );
                }
            }
        }

        Ok(messages)
    }

    /// Full atomic save (for compaction or export).
    pub fn save_agent_snapshot(
        &self,
        agent_id: &str,
        messages: &[ChatMessage],
    ) -> Result<(), SessionError> {
        let jsonl_path = self.agent_file_path(agent_id);
        let temp_path = jsonl_path.with_extension("jsonl.tmp");

        let mut content = String::new();
        for msg in messages {
            let record = SessionRecord::Message {
                message: PersistedMessage {
                    id: msg.id.clone(),
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                    tool_calls: msg.tool_calls.clone(),
                    timestamp_ms: msg.timestamp_ms,
                },
            };
            content.push_str(&serde_json::to_string(&record)?);
            content.push('\n');
        }

        fs::write(&temp_path, &content)?;
        fs::rename(&temp_path, &jsonl_path)?;

        Ok(())
    }

    /// Save a compacted history with a compaction record.
    /// Uses atomic write (not append) because it replaces the entire file.
    pub fn save_compacted_snapshot(
        &mut self,
        agent_id: &str,
        compacted_messages: &[ChatMessage],
        removed_count: usize,
        summary: &str,
    ) -> Result<(), SessionError> {
        let jsonl_path = self.agent_file_path(agent_id);
        let temp_path = jsonl_path.with_extension("jsonl.tmp");

        let mut content = String::new();

        // Compaction record first
        let compaction_record = SessionRecord::Compaction {
            count: 1,
            removed_message_count: removed_count,
            summary: summary.to_string(),
        };
        content.push_str(&serde_json::to_string(&compaction_record)?);
        content.push('\n');

        // Compacted messages
        for msg in compacted_messages {
            let record = SessionRecord::Message {
                message: PersistedMessage {
                    id: msg.id.clone(),
                    role: msg.role.clone(),
                    content: msg.content.clone(),
                    tool_calls: msg.tool_calls.clone(),
                    timestamp_ms: msg.timestamp_ms,
                },
            };
            content.push_str(&serde_json::to_string(&record)?);
            content.push('\n');
        }

        // Atomic write
        fs::write(&temp_path, &content)?;
        fs::rename(&temp_path, &jsonl_path)?;

        // Update meta
        self.meta.updated_at_ms = current_time_millis();
        self.save_meta()?;

        Ok(())
    }

    /// List all sessions.
    pub fn list_sessions(base_dir: &Path) -> Result<Vec<SessionMeta>, SessionError> {
        let mut sessions = vec![];

        if !base_dir.exists() {
            return Ok(sessions);
        }

        for entry in fs::read_dir(base_dir)?.flatten() {
            let meta_path = entry.path().join("meta.json");
            if meta_path.exists() {
                if let Ok(content) = fs::read_to_string(&meta_path) {
                    if let Ok(meta) = serde_json::from_str::<SessionMeta>(&content) {
                        sessions.push(meta);
                    }
                }
            }
        }

        sessions.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
        Ok(sessions)
    }

    /// Delete a session directory from disk.
    pub fn delete_session(base_dir: &Path, session_id: &str) -> Result<(), SessionError> {
        let session_dir = base_dir.join(session_id);
        if !session_dir.exists() {
            return Err(SessionError::NotFound(session_id.to_string()));
        }
        fs::remove_dir_all(&session_dir)?;
        Ok(())
    }

    pub fn meta(&self) -> &SessionMeta {
        &self.meta
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Returns the directory for the current session.
    /// Used by ToolExecutor to persist large tool results.
    pub fn current_session_dir(&self) -> PathBuf {
        self.base_dir.join(&self.current_session_id)
    }

    // ── Private helpers ──

    fn session_dir(&self) -> PathBuf {
        self.base_dir.join(&self.current_session_id)
    }

    fn agent_file_path(&self, agent_id: &str) -> PathBuf {
        self.session_dir().join(format!("{}.jsonl", agent_id))
    }

    fn save_meta(&self) -> Result<(), SessionError> {
        let meta_path = self.session_dir().join("meta.json");
        let temp_path = meta_path.with_extension("json.tmp");
        let meta_json = serde_json::to_string_pretty(&self.meta)?;
        fs::write(&temp_path, &meta_json)?;
        fs::rename(&temp_path, &meta_path)?;
        Ok(())
    }
}

fn generate_session_id() -> String {
    let short_id = &uuid::Uuid::new_v4().as_simple().to_string()[..8];
    format!("session-{}-{}", current_time_millis(), short_id)
}
