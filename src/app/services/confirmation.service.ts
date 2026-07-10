import { Injectable, signal, computed } from '@angular/core';
import type {
  ConfirmationRequest,
  ConfirmationDecision,
} from '../models/chat.model';

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

/**
 * Human-in-the-loop confirmation broker.
 *
 * The chat stream emits `confirmation_required` events when the agent is
 * about to invoke a destructive tool (or a hook returned `Ask`). Those events
 * land here via {@link enqueue}. The modal component reads {@link current}
 * and calls {@link resolve} once the user picks Allow or Deny — that fires
 * the Tauri command which unblocks the parked chat loop.
 *
 * Pending requests queue up so two consecutive tool calls don't race for the
 * same modal slot.
 */
@Injectable({ providedIn: 'root' })
export class ConfirmationService {
  private readonly queue = signal<ConfirmationRequest[]>([]);

  /** The request currently shown to the user (front of the queue). */
  readonly current = computed<ConfirmationRequest | null>(() => {
    const q = this.queue();
    return q.length > 0 ? q[0] : null;
  });

  /** Whether any confirmation modal should be visible. */
  readonly hasPending = computed(() => this.queue().length > 0);

  /** Push a new request onto the queue. Idempotent on call_id. */
  enqueue(req: ConfirmationRequest): void {
    this.queue.update((q) => {
      if (q.some((r) => r.call_id === req.call_id)) return q;
      return [...q, req];
    });
  }

  /**
   * Resolve the front-of-queue request with the user's decision.
   *
   * Removes the request from the queue regardless of whether the Tauri
   * command succeeds — a second resolve attempt for the same call_id is
   * idempotent on the backend (the entry was already removed from the map).
   */
  async resolve(callId: string, decision: ConfirmationDecision): Promise<void> {
    this.queue.update((q) => q.filter((r) => r.call_id !== callId));

    if (!isTauriEnv()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('resolve_tool_confirmation', { callId, decision });
    } catch (err) {
      console.error(`Failed to resolve confirmation ${callId}:`, err);
    }
  }

  /**
   * Drop a request from the queue without calling the backend — used when
   * the chat loop already echoed a `confirmation_resolved` (e.g. the loop
   * timed out and resolved itself with Deny).
   */
  drop(callId: string): void {
    this.queue.update((q) => q.filter((r) => r.call_id !== callId));
  }
}
