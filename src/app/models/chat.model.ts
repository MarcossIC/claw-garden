/** Message returned by the Rust backend (no UI fields). */
export interface BackendMessage {
  id: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  tool_calls?: ToolCall[];
  timestamp_ms: number;
}

export interface ToolCall {
  id: string;
  function: { name: string; arguments: unknown };
}

export type AnomalyType =
  | 'unintended_action'
  | 'reasoning_mismatch'
  | 'prompt_injection_suspicion'
  | 'irreversible_action';

export interface AnomalyData {
  report_id: string;
  anomaly_type: AnomalyType;
  description: string;
  intended_action?: string;
}

/** Frontend message — extends backend shape with UI-only fields. */
export interface ChatMessage {
  id: string;
  agentId: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  timestamp: number;
  isStreaming?: boolean;
  toolUse?: string;
  /** True for synthetic compaction-notification messages inserted by the frontend. */
  isCompaction?: boolean;
  /** When set, the assistant message is an error and this holds the user text that triggered it. */
  failedUserMessage?: string;
  /** Set when this message is a synthetic anomaly self-report (12-B). */
  anomaly?: AnomalyData;
}

export type ConfirmationSource = 'destructive_tool' | 'hook_ask';
export type ConfirmationDecision = 'allow' | 'deny';

export interface ConfirmationRequest {
  call_id: string;
  tool_name: string;
  arguments: unknown;
  reason?: string;
  source: ConfirmationSource;
}

export type EmbedderDownloadPhase =
  | 'start'
  | 'downloading'
  | 'verifying'
  | 'done'
  | 'error';

export type StreamEvent =
  | { kind: 'delta'; delta: string }
  | { kind: 'state'; state: string }
  | { kind: 'done'; full_response: string }
  | { kind: 'error'; error: string }
  | { kind: 'tool_use'; tool_name: string; arguments: string }
  | { kind: 'compaction'; removed_count: number; estimated_tokens_freed: number }
  | { kind: 'usage'; input_tokens: number; output_tokens: number; cache_creation_tokens: number; cache_read_tokens: number }
  | ({ kind: 'confirmation_required' } & ConfirmationRequest)
  | { kind: 'confirmation_resolved'; call_id: string; decision: ConfirmationDecision }
  | ({ kind: 'anomaly_reported' } & AnomalyData)
  | {
      kind: 'embedder_download';
      phase: EmbedderDownloadPhase;
      bytes_downloaded?: number;
      bytes_total?: number;
      error?: string;
    };
