use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use futures::StreamExt;

use super::errors::ToolErrorMessage;
use super::progress::ToolProgress;

// ── read_file ─────────────────────────────────────────────────────────────────

pub async fn read_file(input: &Value) -> Result<String, ToolErrorMessage> {
    let path_str = input["path"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("read_file", "path must be a string"))?;
    let path = PathBuf::from(path_str);

    if !path.exists() {
        return Err(ToolErrorMessage::not_found(path_str));
    }

    // Binary detection: scan first 8KB for NUL bytes
    {
        let mut file = tokio::fs::File::open(&path).await
            .map_err(|e| ToolErrorMessage::io_error("read_file", &e.to_string()))?;
        let meta_len = tokio::fs::metadata(&path).await
            .map(|m| m.len() as usize)
            .unwrap_or(8192);
        let buf_size = 8192.min(meta_len);
        let mut buf = vec![0u8; buf_size];
        let n = file.read(&mut buf).await
            .map_err(|e| ToolErrorMessage::io_error("read_file", &e.to_string()))?;
        if buf[..n].contains(&0u8) {
            return Err(ToolErrorMessage::invalid_input(
                "read_file",
                "file appears to be binary — use bash to handle binary files",
            ));
        }
    }

    let content = tokio::fs::read_to_string(&path).await
        .map_err(|e| ToolErrorMessage::io_error("read_file", &e.to_string()))?;

    let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = input.get("limit").and_then(|v| v.as_u64());

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    // Safe early return for empty file or out-of-range offset.
    // Without this, &lines[offset..] panics when offset >= total.
    if offset >= total {
        if total == 0 {
            return Ok(String::new()); // empty file — not an error
        }
        return Ok(format!(
            "[File has {total} lines, offset {offset} is out of range]"
        ));
    }

    let slice = if let Some(lim) = limit {
        let end = (offset + lim as usize).min(total);
        &lines[offset..end]
    } else {
        &lines[offset..]
    };

    // Format with line numbers (1-indexed)
    let result: String = slice
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}\t{}", offset + i + 1, line))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(result)
}

// ── write_file ────────────────────────────────────────────────────────────────

pub async fn write_file(input: &Value) -> Result<String, ToolErrorMessage> {
    let path_str = input["path"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("write_file", "path must be a string"))?;
    let content = input["content"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("write_file", "content must be a string"))?;

    let path = PathBuf::from(path_str);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await
                .map_err(|e| ToolErrorMessage::io_error("write_file", &e.to_string()))?;
        }
    }

    tokio::fs::write(&path, content).await
        .map_err(|e| ToolErrorMessage::io_error("write_file", &e.to_string()))?;

    Ok(format!("Successfully wrote {} bytes to {}", content.len(), path_str))
}

// ── edit_file ─────────────────────────────────────────────────────────────────

pub async fn edit_file(input: &Value) -> Result<String, ToolErrorMessage> {
    let path_str = input["path"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("edit_file", "path must be a string"))?;
    let old_str = input["old_string"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("edit_file", "old_string must be a string"))?;
    let new_str = input["new_string"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("edit_file", "new_string must be a string"))?;

    let path = PathBuf::from(path_str);
    if !path.exists() {
        return Err(ToolErrorMessage::not_found(path_str));
    }

    let content = tokio::fs::read_to_string(&path).await
        .map_err(|e| ToolErrorMessage::io_error("edit_file", &e.to_string()))?;

    let count = content.matches(old_str).count();
    if count == 0 {
        return Err(ToolErrorMessage::invalid_input(
            "edit_file",
            &format!("old_string not found in {path_str}. Make sure it matches exactly."),
        ));
    }
    if count > 1 {
        return Err(ToolErrorMessage::invalid_input(
            "edit_file",
            &format!("old_string found {count} times in {path_str}. It must be unique — add more context lines to disambiguate."),
        ));
    }

    let new_content = content.replacen(old_str, new_str, 1);
    tokio::fs::write(&path, &new_content).await
        .map_err(|e| ToolErrorMessage::io_error("edit_file", &e.to_string()))?;

    Ok(format!("Successfully edited {path_str}"))
}

// ── glob_search ───────────────────────────────────────────────────────────────

pub async fn glob_search(input: &Value) -> Result<String, ToolErrorMessage> {
    let pattern_str = input["pattern"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("glob_search", "pattern must be a string"))?;
    let base_path = input.get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let base = PathBuf::from(base_path);

    if !base.exists() {
        return Err(ToolErrorMessage::not_found(base_path));
    }

    let glob = globset::GlobBuilder::new(pattern_str)
        .case_insensitive(false)
        .literal_separator(true) // *.rs = top-level only; **/*.rs = recursive (standard shell behavior)
        .build()
        .map_err(|e| ToolErrorMessage::invalid_input("glob_search", &format!("invalid glob pattern: {e}")))?
        .compile_matcher();

    let mut matches = Vec::new();
    walk_and_glob(&base, &base, &glob, &mut matches, 0)?;
    matches.sort();

    if matches.is_empty() {
        return Ok(format!("No files matched pattern '{pattern_str}' in '{base_path}'"));
    }

    Ok(matches.join("\n"))
}

fn walk_and_glob(
    base: &Path,
    current: &Path,
    glob: &globset::GlobMatcher,
    matches: &mut Vec<String>,
    depth: usize,
) -> Result<(), ToolErrorMessage> {
    if depth > 50 {
        return Ok(()); // Depth limit to prevent runaway
    }
    let entries = std::fs::read_dir(current)
        .map_err(|e| ToolErrorMessage::io_error("glob_search", &e.to_string()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path.strip_prefix(base).unwrap_or(&path);
        let relative_str = relative.to_string_lossy();

        if glob.is_match(&*relative_str) {
            matches.push(relative_str.into_owned());
        }

        if path.is_dir() {
            // Skip hidden directories and common noise
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with('.') && name != "node_modules" && name != "target" {
                let _ = walk_and_glob(base, &path, glob, matches, depth + 1);
            }
        }
    }
    Ok(())
}

// ── grep_search ───────────────────────────────────────────────────────────────

pub async fn grep_search(input: &Value) -> Result<String, ToolErrorMessage> {
    let pattern_str = input["pattern"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("grep_search", "pattern must be a string"))?;
    let search_path = input.get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let file_pattern = input.get("file_pattern").and_then(|v| v.as_str());
    let case_insensitive = input.get("case_insensitive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let base = PathBuf::from(search_path);
    if !base.exists() {
        return Err(ToolErrorMessage::not_found(search_path));
    }

    // Build regex
    let re = {
        let mut builder = regex::RegexBuilder::new(pattern_str);
        builder.case_insensitive(case_insensitive);
        builder.build().map_err(|e| ToolErrorMessage::invalid_input(
            "grep_search",
            &format!("invalid regex: {e}"),
        ))?
    };

    // Optional file glob filter
    let file_glob = file_pattern.and_then(|p| {
        globset::GlobBuilder::new(p)
            .case_insensitive(false)
            .build()
            .ok()
            .map(|g| g.compile_matcher())
    });

    let mut results = Vec::new();
    walk_and_grep(&base, &base, &re, file_glob.as_ref(), &mut results, 0)?;

    if results.is_empty() {
        return Ok(format!("No matches found for pattern '{pattern_str}' in '{search_path}'"));
    }

    Ok(results.join("\n"))
}

fn walk_and_grep(
    base: &Path,
    current: &Path,
    re: &regex::Regex,
    file_glob: Option<&globset::GlobMatcher>,
    results: &mut Vec<String>,
    depth: usize,
) -> Result<(), ToolErrorMessage> {
    if depth > 50 {
        return Ok(());
    }
    let entries = std::fs::read_dir(current)
        .map_err(|e| ToolErrorMessage::io_error("grep_search", &e.to_string()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with('.') && name != "node_modules" && name != "target" {
                let _ = walk_and_grep(base, &path, re, file_glob, results, depth + 1);
            }
            continue;
        }

        // Apply file glob filter if provided
        if let Some(glob) = file_glob {
            let relative = path.strip_prefix(base).unwrap_or(&path);
            if !glob.is_match(relative) {
                continue;
            }
        }

        // Skip binary files (simple heuristic: check first 512 bytes for NUL)
        if let Ok(mut f) = std::fs::File::open(&path) {
            use std::io::Read;
            let mut buf = [0u8; 512];
            if let Ok(n) = f.read(&mut buf) {
                if buf[..n].contains(&0u8) {
                    continue;
                }
            }
        }

        // Read and search
        if let Ok(content) = std::fs::read_to_string(&path) {
            let relative = path.strip_prefix(base).unwrap_or(&path);
            for (line_num, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    results.push(format!("{}:{}: {}", relative.display(), line_num + 1, line));
                    if results.len() >= 500 {
                        results.push("[results truncated at 500 matches]".to_string());
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
}

// ── bash ──────────────────────────────────────────────────────────────────────

/// Execute a shell command with line-by-line stdout/stderr streaming.
///
/// When `progress_tx` is provided, each line read from the child's stdout or
/// stderr is emitted as a `ToolProgress::BashProgress` event — this lets the UI
/// animate the agent in real time instead of appearing frozen until the command
/// completes. The final return value is still the full combined output for the
/// tool_result, so callers without a progress channel get the same result as
/// before.
///
/// Timeout enforcement matches the old behavior: the entire operation is capped
/// at `timeout_secs`. If the timeout fires, the child is killed via `kill()`
/// before returning the error so we don't leak orphaned processes.
pub async fn bash(
    input: &Value,
    project_path: Option<&str>,
    progress_tx: Option<&UnboundedSender<ToolProgress>>,
) -> Result<String, ToolErrorMessage> {
    let command = input["command"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("bash", "command must be a string"))?;
    let timeout_override = input.get("timeout_secs").and_then(|v| v.as_u64());
    let timeout_secs = timeout_override.unwrap_or(120).min(300);

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };

    if let Some(path) = project_path {
        cmd.current_dir(path);
    }

    // Piped stdio so we can read line-by-line as the child writes them.
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ToolErrorMessage::io_error("bash", &e.to_string()))?;

    let stdout = child.stdout.take()
        .ok_or_else(|| ToolErrorMessage::io_error("bash", "failed to capture stdout"))?;
    let stderr = child.stderr.take()
        .ok_or_else(|| ToolErrorMessage::io_error("bash", "failed to capture stderr"))?;

    // Two readers run concurrently — they forward chunks to progress_tx AND
    // accumulate the full output for the tool_result. We can't replace the
    // accumulation with a single join later because tokio readers consume the
    // pipe as they go; nothing else can read it afterwards.
    let tx_stdout = progress_tx.cloned();
    let tx_stderr = progress_tx.cloned();

    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        let mut accum = String::new();
        while let Ok(Some(line)) = reader.next_line().await {
            if let Some(tx) = &tx_stdout {
                // Best-effort send — if the UI dropped the receiver, we just
                // keep accumulating for the final result.
                let _ = tx.send(ToolProgress::BashProgress {
                    stdout_chunk: format!("{line}\n"),
                    stderr_chunk: String::new(),
                });
            }
            accum.push_str(&line);
            accum.push('\n');
        }
        accum
    });

    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut accum = String::new();
        while let Ok(Some(line)) = reader.next_line().await {
            if let Some(tx) = &tx_stderr {
                let _ = tx.send(ToolProgress::BashProgress {
                    stdout_chunk: String::new(),
                    stderr_chunk: format!("{line}\n"),
                });
            }
            accum.push_str(&line);
            accum.push('\n');
        }
        accum
    });

    // Wait for the child with a timeout; readers finish naturally when the
    // pipes close on process exit.
    let status = match tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        child.wait(),
    )
    .await
    {
        Ok(result) => result.map_err(|e| ToolErrorMessage::io_error("bash", &e.to_string()))?,
        Err(_) => {
            // Timeout → kill the child so it doesn't outlive the tool call.
            // kill() is a no-op if the process already exited.
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(ToolErrorMessage::timeout("bash", timeout_secs, 1));
        }
    };

    let stdout_str = stdout_task.await.unwrap_or_default();
    let stderr_str = stderr_task.await.unwrap_or_default();

    let exit_code = status.code();

    let mut result = stdout_str.trim_end_matches('\n').to_string();
    let stderr_trimmed = stderr_str.trim_end_matches('\n');
    if !stderr_trimmed.is_empty() {
        if !result.is_empty() { result.push('\n'); }
        result.push_str(&format!("[stderr]\n{stderr_trimmed}"));
    }

    if result.is_empty() {
        if status.success() {
            result = "(no output)".to_string();
        } else {
            return Err(ToolErrorMessage::execution_error(
                "bash",
                &format!("command exited with code {:?}", exit_code),
                exit_code,
            ));
        }
    }

    Ok(result)
}

// ── list_directory ────────────────────────────────────────────────────────────

pub async fn list_directory(input: &Value) -> Result<String, ToolErrorMessage> {
    let path_str = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let path = PathBuf::from(path_str);

    if !path.exists() {
        return Err(ToolErrorMessage::not_found(path_str));
    }

    let mut entries = tokio::fs::read_dir(&path).await
        .map_err(|e| ToolErrorMessage::io_error("list_directory", &e.to_string()))?;

    let mut lines = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().await;
        let (kind, size) = match metadata {
            Ok(m) if m.is_dir() => ("dir".to_string(), String::new()),
            Ok(m) => ("file".to_string(), format!(" ({} bytes)", m.len())),
            Err(_) => ("unknown".to_string(), String::new()),
        };
        lines.push((name.clone(), format!("{kind:5}  {name}{size}")));
    }

    lines.sort_by(|a, b| a.0.cmp(&b.0));
    let output: Vec<String> = lines.into_iter().map(|(_, s)| s).collect();

    if output.is_empty() {
        return Ok(format!("Directory '{path_str}' is empty"));
    }

    Ok(output.join("\n"))
}

// ── web_fetch ─────────────────────────────────────────────────────────────────

/// Fetch a URL and return the response body as text.
///
/// When `progress_tx` is provided, emits a `WebFetchProgress` every time at
/// least `PROGRESS_CHUNK_BYTES` have accumulated since the last event — this
/// keeps the UI informed on large downloads without flooding the channel with
/// one event per TCP segment.
const PROGRESS_CHUNK_BYTES: u64 = 32 * 1024; // 32 KB

pub async fn web_fetch(
    input: &Value,
    client: &reqwest::Client,
    progress_tx: Option<&UnboundedSender<ToolProgress>>,
) -> Result<String, ToolErrorMessage> {
    let url = input["url"].as_str()
        .ok_or_else(|| ToolErrorMessage::invalid_input("web_fetch", "url must be a string"))?;

    let response = tokio::time::timeout(
        Duration::from_secs(30),
        client.get(url)
            .header("User-Agent", "claw-garden/1.0")
            .send(),
    )
    .await
    .map_err(|_| ToolErrorMessage::timeout("web_fetch", 30, 1))?
    .map_err(|e| ToolErrorMessage::io_error("web_fetch", &e.to_string()))?;

    if !response.status().is_success() {
        return Err(ToolErrorMessage::execution_error(
            "web_fetch",
            &format!("HTTP {}", response.status()),
            Some(response.status().as_u16() as i32),
        ));
    }

    // Stream the body in chunks so we can emit progress. The whole download
    // is wrapped in a 30s timeout — matches the previous behavior — but now
    // the timer guards the body read, not just the handshake.
    let url_owned = url.to_string();
    let collect_fut = async {
        let mut body = Vec::new();
        let mut last_reported: u64 = 0;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                ToolErrorMessage::io_error("web_fetch", &e.to_string())
            })?;
            body.extend_from_slice(&chunk);
            if let Some(tx) = progress_tx {
                let total = body.len() as u64;
                if total - last_reported >= PROGRESS_CHUNK_BYTES {
                    let _ = tx.send(ToolProgress::WebFetchProgress {
                        bytes_read: total,
                        url: url_owned.clone(),
                    });
                    last_reported = total;
                }
            }
        }
        Ok::<Vec<u8>, ToolErrorMessage>(body)
    };

    let body = tokio::time::timeout(Duration::from_secs(30), collect_fut)
        .await
        .map_err(|_| ToolErrorMessage::timeout("web_fetch", 30, 1))??;

    // UTF-8 with lossy fallback — matches reqwest::Response::text() which uses
    // the Content-Type charset, but we skip that and accept the occasional
    // replacement char on non-UTF-8 pages. Matches the behavior most tools
    // expect for web scraping.
    Ok(String::from_utf8_lossy(&body).into_owned())
}
