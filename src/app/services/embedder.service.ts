import { DestroyRef, Injectable, inject, signal } from '@angular/core';
import type { EmbedderDownloadPhase, StreamEvent } from '../models/chat.model';

export interface EmbedderStatus {
  phase: 'idle' | EmbedderDownloadPhase;
  bytesDownloaded?: number;
  bytesTotal?: number;
  error?: string;
}

/**
 * Loose shape for the `@tauri-apps/api/event` payload wrapper.
 * Defined locally so tests can construct fake events without importing the real module.
 */
interface TauriEventPayload<T> {
  payload: T;
}

type UnlistenFn = () => void;

/**
 * Function signature compatible with `@tauri-apps/api/core::invoke`.
 * Loose typing keeps the service swappable in tests without pulling the real Tauri runtime.
 */
type InvokeFn = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

@Injectable({ providedIn: 'root' })
export class EmbedderService {
  readonly status = signal<EmbedderStatus>({ phase: 'idle' });

  private readonly destroyRef = inject(DestroyRef);
  private unlisten?: UnlistenFn;

  /**
   * Swappable invoke fn. Default lazy-loads the real Tauri `invoke` in `retry()`.
   * Tests assign a stub via `setInvokeForTest(...)` to avoid the dynamic import.
   */
  private invokeFn?: InvokeFn;

  constructor() {
    this.bootstrap();
    this.destroyRef.onDestroy(() => {
      this.unlisten?.();
      this.unlisten = undefined;
    });
  }

  /**
   * Lazy-subscribes to the `embedder-download` Tauri event.
   * No-op outside the Tauri runtime (browser-only dev mode).
   */
  private async bootstrap(): Promise<void> {
    if (!isTauriEnv()) return;
    try {
      const mod = await import('@tauri-apps/api/event');
      const unlisten = await mod.listen<StreamEvent>('embedder-download', (event) => {
        this.handleEvent(event.payload);
      });
      this.unlisten = unlisten;
    } catch (err) {
      // Surface bootstrap failures (e.g. plugin not registered) without crashing the app.
      console.error('EmbedderService: failed to subscribe to embedder-download events', err);
    }
  }

  /**
   * Trigger a fresh embedder bootstrap on the backend.
   * Optimistically resets status to `{ phase: 'start' }` before invoking
   * so the UI immediately reflects the user's intent.
   */
  async retry(): Promise<void> {
    this.status.set({ phase: 'start' });

    const invoker = await this.resolveInvoke();
    if (!invoker) return;

    try {
      await invoker('retry_embedder_download');
    } catch (err) {
      this.status.set({ phase: 'error', error: String(err) });
    }
  }

  private async resolveInvoke(): Promise<InvokeFn | undefined> {
    if (this.invokeFn) return this.invokeFn;
    if (!isTauriEnv()) return undefined;
    const mod = await import('@tauri-apps/api/core');
    return mod.invoke as InvokeFn;
  }

  // ─── Test seams (NOT public API) ──────────────────────────────────────────

  /**
   * Drive a synthetic StreamEvent through the same signal-update path that the
   * Tauri `listen` callback uses. Test-only entry point.
   * @internal
   */
  handleEventForTest(ev: StreamEvent): void {
    this.handleEvent(ev);
  }

  /**
   * Override the `invoke` function used by `retry()`. Test-only entry point.
   * @internal
   */
  setInvokeForTest(fn: InvokeFn): void {
    this.invokeFn = fn;
  }

  /**
   * Drive a synthetic Tauri event payload through the bootstrap callback.
   * Useful for tests that want to assert on the `Event<T>.payload` unwrapping.
   * @internal
   */
  handleListenPayloadForTest(event: TauriEventPayload<StreamEvent>): void {
    this.handleEvent(event.payload);
  }

  // ─── Internal ─────────────────────────────────────────────────────────────

  private handleEvent(ev: StreamEvent): void {
    if (ev.kind !== 'embedder_download') return;

    const next: EmbedderStatus = { phase: ev.phase };
    if (typeof ev.bytes_downloaded === 'number') {
      next.bytesDownloaded = ev.bytes_downloaded;
    }
    if (typeof ev.bytes_total === 'number') {
      next.bytesTotal = ev.bytes_total;
    }
    if (typeof ev.error === 'string') {
      next.error = ev.error;
    }

    this.status.set(next);
  }
}
