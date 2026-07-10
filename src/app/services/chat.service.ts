import { Injectable, signal, computed, effect, inject } from '@angular/core';
import type { ChatMessage, BackendMessage, StreamEvent, AnomalyData } from '../models/chat.model';
import { eventBus } from '../game/event-bus';
import { ConfirmationService } from './confirmation.service';

/** Temporary display-only ID for optimistic UI during streaming. */
let displayCounter = 0;
function tempDisplayId(): string {
  return `tmp-${++displayCounter}-${Date.now()}`;
}

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

@Injectable({ providedIn: 'root' })
export class ChatService {
  private readonly confirmationService = inject(ConfirmationService);
  private readonly allMessages = signal<ChatMessage[]>([]);
  private readonly loadedAgents = new Set<string>();

  readonly isStreaming = signal(false);
  readonly activeAgentId = signal<string | null>(null);

  readonly messages = computed(() => {
    const agentId = this.activeAgentId();
    if (!agentId) return [];
    return this.allMessages().filter((m) => m.agentId === agentId);
  });

  constructor() {
    // When active agent changes, load their history from backend
    effect(() => {
      const agentId = this.activeAgentId();
      if (agentId && !this.loadedAgents.has(agentId)) {
        this.loadAgentHistory(agentId);
      }
    });
  }

  /** Load persisted messages for an agent from the backend. */
  async loadAgentHistory(agentId: string): Promise<void> {
    if (!isTauriEnv()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const backendMessages = await invoke<BackendMessage[]>('get_agent_messages', { agentId });

      if (backendMessages.length === 0) {
        this.loadedAgents.add(agentId);
        return;
      }

      const mapped: ChatMessage[] = backendMessages.map((m) => ({
        id: m.id,
        agentId,
        role: m.role === 'tool' ? 'system' : m.role,
        content: m.content,
        timestamp: m.timestamp_ms,
      }));

      // Replace this agent's messages with backend truth,
      // but keep synthetic compaction indicators for UI continuity
      this.allMessages.update((msgs) => {
        const compactionMsgs = msgs.filter((m) => m.agentId === agentId && m.isCompaction);
        const otherAgentMsgs = msgs.filter((m) => m.agentId !== agentId);
        return [...otherAgentMsgs, ...mapped, ...compactionMsgs];
      });
      this.loadedAgents.add(agentId);
    } catch (err) {
      console.error(`Failed to load history for agent ${agentId}:`, err);
    }
  }

  /** Clear loaded state so next agent select reloads from backend. */
  clearLoadedState(): void {
    this.loadedAgents.clear();
    this.allMessages.set([]);
  }

  async sendMessage(agentId: string, text: string): Promise<void> {
    if (this.isStreaming() || !text.trim()) return;

    // Optimistic user message for immediate UI feedback
    const userMsg: ChatMessage = {
      id: tempDisplayId(),
      agentId,
      role: 'user',
      content: text.trim(),
      timestamp: Date.now(),
    };
    this.allMessages.update((msgs) => [...msgs, userMsg]);

    if (isTauriEnv()) {
      await this.streamFromTauri(agentId, text.trim());
    } else {
      await this.streamMock(agentId);
    }
  }

  private async streamFromTauri(agentId: string, message: string, isRetry = false): Promise<void> {
    this.isStreaming.set(true);

    const assistantMsg: ChatMessage = {
      id: tempDisplayId(),
      agentId,
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
      isStreaming: true,
    };
    this.allMessages.update((msgs) => [...msgs, assistantMsg]);
    const assistantMsgId = assistantMsg.id;

    try {
      const { invoke, Channel } = await import('@tauri-apps/api/core');
      const channel = new Channel<StreamEvent>();

      channel.onmessage = (event: StreamEvent) => {
        switch (event.kind) {
          case 'state':
            this.handleStateEvent(agentId, event.state);
            break;
          case 'delta':
            this.appendToMessage(assistantMsgId, event.delta);
            break;
          case 'tool_use':
            this.handleToolUse(agentId, assistantMsgId, event.tool_name, event.arguments);
            break;
          case 'done':
            this.finalizeMessage(assistantMsgId);
            this.handleDone(agentId);
            break;
          case 'error':
            this.handleError(assistantMsgId, event.error, message);
            this.handleDone(agentId);
            break;
          case 'compaction':
            this.handleCompaction(agentId, event.removed_count, event.estimated_tokens_freed);
            break;
          case 'confirmation_required':
            this.confirmationService.enqueue({
              call_id: event.call_id,
              tool_name: event.tool_name,
              arguments: event.arguments,
              reason: event.reason,
              source: event.source,
            });
            break;
          case 'confirmation_resolved':
            // Backend echo — modal already closed when the user clicked.
            // Drop just in case the loop self-resolved (e.g. fail-closed Deny).
            this.confirmationService.drop(event.call_id);
            break;
          case 'anomaly_reported':
            this.handleAnomalyReported(agentId, event);
            break;
        }
      };

      await invoke('stream_chat', { agentId, message, isRetry: isRetry || undefined, onEvent: channel });
    } catch (err) {
      this.handleError(assistantMsgId, String(err), message);
      this.handleDone(agentId);
    }
  }

  private async streamMock(agentId: string): Promise<void> {
    this.isStreaming.set(true);

    const assistantMsg: ChatMessage = {
      id: tempDisplayId(),
      agentId,
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
      isStreaming: true,
    };
    this.allMessages.update((msgs) => [...msgs, assistantMsg]);
    const msgId = assistantMsg.id;

    eventBus.emit('agent-state-changed', {
      agentId,
      state: { kind: 'Thinking', taskSummary: 'Processing...' },
    });

    await this.delay(800);

    eventBus.emit('agent-state-changed', {
      agentId,
      state: { kind: 'Coding' },
    });

    const mockResponse =
      "This is a mock response since Ollama isn't connected. " +
      'The real streaming will show tokens one by one as they arrive from the LLM.';

    for (const char of mockResponse) {
      this.appendToMessage(msgId, char);
      await this.delay(20);
    }

    this.finalizeMessage(msgId);
    this.handleDone(agentId);
  }

  private handleStateEvent(agentId: string, state: string): void {
    if (state === 'thinking') {
      eventBus.emit('agent-state-changed', {
        agentId,
        state: { kind: 'Thinking', taskSummary: 'Processing...' },
      });
    } else if (state === 'typing') {
      eventBus.emit('agent-state-changed', {
        agentId,
        state: { kind: 'Coding' },
      });
    } else if (state === 'reading') {
      eventBus.emit('agent-state-changed', {
        agentId,
        state: { kind: 'Reading', source: 'file' },
      });
    } else if (state === 'awaiting_confirmation') {
      eventBus.emit('agent-state-changed', {
        agentId,
        state: { kind: 'Thinking', taskSummary: 'Awaiting your approval...' },
      });
    }
  }

  private handleToolUse(agentId: string, msgId: string, toolName: string, args: string): void {
    let label = toolName;
    try {
      const parsed = JSON.parse(args);
      if (parsed.path) label = `${toolName}(${parsed.path})`;
    } catch {
      // Use raw tool name if args aren't JSON
    }

    this.allMessages.update((msgs) =>
      msgs.map((m) => (m.id === msgId ? { ...m, toolUse: label } : m)),
    );
  }

  private handleAnomalyReported(agentId: string, data: AnomalyData): void {
    const msg: ChatMessage = {
      id: tempDisplayId(),
      agentId,
      role: 'system',
      content: data.description,
      timestamp: Date.now(),
      anomaly: {
        report_id: data.report_id,
        anomaly_type: data.anomaly_type,
        description: data.description,
        intended_action: data.intended_action,
      },
    };
    this.allMessages.update((msgs) => [...msgs, msg]);
  }

  private handleCompaction(agentId: string, removedCount: number, tokenFreed: number): void {
    // Emit visual state for office scene animation
    eventBus.emit('agent-state-changed', {
      agentId,
      state: { kind: 'Thinking', taskSummary: 'Organizing notes...' },
    });

    // Insert a subtle system message in chat
    const compactionMsg: ChatMessage = {
      id: tempDisplayId(),
      agentId,
      role: 'system',
      content: `Context compacted: ${removedCount} messages archived, ~${Math.round(tokenFreed / 1000)}k tokens freed.`,
      timestamp: Date.now(),
      isCompaction: true,
    };
    this.allMessages.update((msgs) => [...msgs, compactionMsg]);

    // Reload the agent's history from backend to stay in sync
    this.loadedAgents.delete(agentId);
    this.loadAgentHistory(agentId);
  }

  private handleDone(agentId: string): void {
    this.isStreaming.set(false);
    eventBus.emit('agent-state-changed', {
      agentId,
      state: { kind: 'Idle' },
    });
  }

  private handleError(msgId: string, error: string, userMessage?: string): void {
    this.allMessages.update((msgs) =>
      msgs.map((m) =>
        m.id === msgId
          ? { ...m, content: `Error: ${error}`, isStreaming: false, failedUserMessage: userMessage }
          : m,
      ),
    );
  }

  /** Retry the exact same message that caused the error. */
  async retryMessage(errorMsgId: string): Promise<void> {
    const errorMsg = this.allMessages().find((m) => m.id === errorMsgId);
    if (!errorMsg?.failedUserMessage || this.isStreaming()) return;

    const agentId = errorMsg.agentId;
    const text = errorMsg.failedUserMessage;

    // Remove only the failed assistant message — keep the user message in place
    this.allMessages.update((msgs) => msgs.filter((m) => m.id !== errorMsgId));

    // Stream again with isRetry=true so backend skips re-persisting the user message
    if (isTauriEnv()) {
      await this.streamFromTauri(agentId, text, true);
    } else {
      await this.streamMock(agentId);
    }
  }

  /** Load the failed message text into input for editing, removing the failed pair. */
  editFailedMessage(errorMsgId: string): string | null {
    const errorMsg = this.allMessages().find((m) => m.id === errorMsgId);
    if (!errorMsg?.failedUserMessage || this.isStreaming()) return null;

    const text = errorMsg.failedUserMessage;
    this.removeFailedPair(errorMsgId);
    return text;
  }

  /** Remove a failed assistant message and the user message right before it. */
  private removeFailedPair(errorMsgId: string): void {
    this.allMessages.update((msgs) => {
      const errorIdx = msgs.findIndex((m) => m.id === errorMsgId);
      if (errorIdx < 0) return msgs;

      // The user message is the one right before the error assistant message
      const userIdx = errorIdx > 0 && msgs[errorIdx - 1].role === 'user' ? errorIdx - 1 : -1;

      return msgs.filter((_, i) => i !== errorIdx && i !== userIdx);
    });
  }

  private appendToMessage(msgId: string, delta: string): void {
    this.allMessages.update((msgs) =>
      msgs.map((m) => (m.id === msgId ? { ...m, content: m.content + delta } : m)),
    );
  }

  private finalizeMessage(msgId: string): void {
    this.allMessages.update((msgs) =>
      msgs.map((m) => (m.id === msgId ? { ...m, isStreaming: false } : m)),
    );
  }

  private delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }
}
