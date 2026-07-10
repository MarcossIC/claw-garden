import { Injectable, signal } from '@angular/core';
import type { WorkspaceConfig } from '../models/workspace.model';

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

@Injectable({ providedIn: 'root' })
export class WorkspaceConfigService {
  readonly config = signal<WorkspaceConfig>({});
  readonly loading = signal(false);
  readonly saving = signal(false);
  /** Project path for which `config` is currently loaded. */
  readonly loadedPath = signal<string | null>(null);

  /**
   * Monotonically-increasing counter used to implement "latest wins" semantics.
   * Each call to `loadForPath` increments this counter and captures its value.
   * When the async call resolves, it only commits state if its captured value is
   * still the current counter — i.e., no newer load was started after it.
   */
  private loadSeq = 0;

  /** Load workspace config for the given project path from disk. */
  async loadForPath(projectPath: string): Promise<void> {
    const trimmed = projectPath.trim();
    if (!trimmed) {
      this.config.set({});
      this.loadedPath.set(null);
      return;
    }
    // Skip reload if this path is already loaded.
    if (this.loadedPath() === trimmed) return;

    const seq = ++this.loadSeq;
    this.loading.set(true);

    if (!isTauriEnv()) {
      this.config.set({});
      this.loadedPath.set(trimmed);
      this.loading.set(false);
      return;
    }

    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const cfg = await invoke<WorkspaceConfig>('get_workspace_config', {
        projectPath: trimmed,
      });
      // Discard result if a newer load superseded this one.
      if (seq !== this.loadSeq) return;
      this.config.set(cfg);
      this.loadedPath.set(trimmed);
    } catch (err) {
      if (seq !== this.loadSeq) return;
      console.error('Failed to load workspace config:', err);
      this.config.set({});
      // Mark the path as attempted so we don't retry on every keystroke.
      this.loadedPath.set(trimmed);
    } finally {
      if (seq === this.loadSeq) {
        this.loading.set(false);
      }
    }
  }

  /** Persist workspace config for the given project path. */
  async saveForPath(projectPath: string, config: WorkspaceConfig): Promise<void> {
    const trimmed = projectPath.trim();
    if (!trimmed) return;

    if (!isTauriEnv()) {
      this.config.set(config);
      this.loadedPath.set(trimmed);
      return;
    }

    this.saving.set(true);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('set_workspace_config', {
        projectPath: trimmed,
        providerId: config.provider_id ?? null,
        model: config.model ?? null,
      });
      this.config.set(config);
      this.loadedPath.set(trimmed);
    } catch (err) {
      console.error('Failed to save workspace config:', err);
      throw err;
    } finally {
      this.saving.set(false);
    }
  }

  /** Clear loaded state (e.g. when the project path is cleared). */
  reset(): void {
    this.config.set({});
    this.loadedPath.set(null);
  }
}
