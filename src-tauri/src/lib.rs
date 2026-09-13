mod commands;
mod compaction;
mod mcp;
mod memory_runtime;
mod tools;
mod recovery;
mod session;
mod state;

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use clawg_core::memory::bootstrap::{
    production_deps, run_embedder_bootstrap, EmbedderEventSink,
};
use clawg_core::models::chat::StreamEvent;
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (state, memory_rx) = AppState::new();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(move |app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            // Spawn the embedder bootstrap task. It downloads the model in the
            // background and forwards progress events via
            // `app.emit("embedder-download", …)`. Non-blocking — failure
            // leaves the OnceCell empty and the rest of the app keeps running.
            start_embedder_bootstrap(app.handle().clone());

            // Spawn the memory ingest worker. Receives MemoryEvents from
            // push_and_persist and writes rows to memory + memory_vectors.
            // memory_rx is moved here and consumed by the worker task for the
            // app lifetime.
            crate::memory_runtime::start_memory_worker(app.handle().clone(), memory_rx);

            // Access tracker lifecycle hook. The flush worker is already
            // running (spawned inside AccessTracker::new() during
            // AppState::new()). This call exists for lifecycle parity with
            // start_memory_worker and is the extension point for a future
            // supervision/reconnect mechanism, should the flush worker ever
            // need one.
            start_access_worker(app.handle().clone());

            // memoria-3-capas-fase-3: spawn the maintenance job (periodic
            // inactivity-based tier demotion). First pass fires 5 minutes
            // after startup, then every 24h — see `memory::maintenance`.
            crate::memory_runtime::start_maintenance_worker(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::agents::get_agents,
            commands::chat::stream_chat,
            commands::config::set_project_path,
            commands::config::get_project_path,
            commands::config::get_config,
            commands::config::set_provider,
            commands::config::set_aux_llm_flags,
            commands::config::check_ollama,
            commands::config::openrouter_list_models,
            commands::config::get_auth_status,
            commands::auth::oauth_status,
            commands::auth::oauth_login,
            commands::auth::oauth_logout,
            commands::auth::oauth_refresh,
            commands::providers::list_providers,
            commands::providers::add_provider,
            commands::providers::update_provider,
            commands::providers::delete_provider,
            commands::agent_crud::create_agent,
            commands::agent_crud::update_agent,
            commands::agent_crud::delete_agent,
            commands::session::list_sessions,
            commands::session::current_session,
            commands::session::new_session,
            commands::session::resume_session,
            commands::session::delete_session,
            commands::session::get_agent_messages,
            commands::workspace::get_workspace_config,
            commands::workspace::set_workspace_config,
            commands::confirmations::resolve_tool_confirmation,
            commands::embedder::retry_embedder_download,
            commands::memory_admin::list_memories,
            commands::memory_admin::set_importance,
            commands::memory_admin::delete_memory,
            commands::memory_admin::get_skill_steps_trend,
            commands::memory_admin::get_failure_histogram,
            commands::memory_admin::reembed_all,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ============================================================================
// Access tracker lifecycle hook
// ============================================================================

/// Lifecycle hook called from `run()` after `start_memory_worker`.
///
/// The access tracker flush worker is already running — it is spawned inside
/// [`clawg_core::memory::access::AccessTracker::new`] during `AppState::new()`.
/// This function exists for lifecycle parity with `start_memory_worker` and
/// is the extension point for a future supervision/reconnect mechanism,
/// should the flush worker ever need one.
fn start_access_worker(_app_handle: AppHandle) {
    // No-op today: the worker is already alive for the duration of AppState,
    // spawned by AccessTracker::new() rather than by this hook.
}

// ============================================================================
// Embedder bootstrap orchestration
// ============================================================================
//
// The lifecycle itself (download → load → contract check → fill the cell)
// lives in `clawg_core::memory::bootstrap`, where the headless hosts can reach
// it too. What stays here is the only genuinely Tauri-shaped part: forwarding
// each `StreamEvent` onto the app's event bus, and choosing the executor.

/// Production sink: forwards to `AppHandle::emit("embedder-download", …)`.
///
/// The desktop half of [`EmbedderEventSink`] — the daemon uses
/// `clawg_core::memory::bootstrap::LogSink` instead. Emission failure is
/// deliberately ignored: it means no window is listening (shutdown, or a
/// window not yet created), which must not abort a download in flight.
struct AppHandleSink {
    app: AppHandle,
}

impl EmbedderEventSink for AppHandleSink {
    fn emit_event(&self, event: StreamEvent) {
        let _ = self.app.emit("embedder-download", &event);
    }
}

/// Public entry point used by both the setup hook and the
/// [`commands::embedder::retry_embedder_download`] command. Spawns the
/// embedder bootstrap on `tauri::async_runtime` and returns immediately.
///
/// Resolves the three handles the bootstrap needs out of [`AppState`], wraps
/// the `AppHandle` in a sink, and hands everything to the core. Errors inside
/// the task are never propagated — on any failure the cell stays empty, which
/// is a defensible state the retry command can act on, and the app keeps
/// running.
pub(crate) fn start_embedder_bootstrap(app_handle: AppHandle) {
    let state = app_handle.state::<AppState>();
    let base_dir = state.base_dir.clone();
    let embedder_cell = state.embedder.clone();
    let (ensure, load, verify) = production_deps(state.memory_store.clone());
    let sink = Arc::new(AppHandleSink {
        app: app_handle.clone(),
    });

    tauri::async_runtime::spawn(async move {
        run_embedder_bootstrap(base_dir, embedder_cell, sink, ensure, load, verify).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AppHandle` cannot be constructed without a real Tauri runtime, so the
    /// bootstrap lifecycle is asserted in `clawg_core::memory::bootstrap`
    /// against its injectable seams. What is verifiable here is the wiring
    /// contract this file is responsible for: the sink it supplies must
    /// satisfy the bound the core demands, or `start_embedder_bootstrap` could
    /// not have compiled.
    #[test]
    fn app_handle_sink_satisfies_the_core_sink_bound() {
        fn _accepts<T: EmbedderEventSink>() {}
        _accepts::<AppHandleSink>();
        fn _is_object_safe_here(s: Arc<AppHandleSink>) -> Arc<dyn EmbedderEventSink> {
            s
        }
    }
}
