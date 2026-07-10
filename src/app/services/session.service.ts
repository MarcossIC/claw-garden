import { Injectable, signal, computed, inject } from '@angular/core';
import { ChatService } from './chat.service';

export interface SessionMeta {
  version: number;
  session_id: string;
  created_at_ms: number;
  updated_at_ms: number;
  agents: string[];
}

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

@Injectable({ providedIn: 'root' })
export class SessionService {
  private readonly chatService = inject(ChatService);
  private readonly _sessions = signal<SessionMeta[]>([]);
  private readonly _currentSession = signal<SessionMeta | null>(null);

  readonly sessions = this._sessions.asReadonly();
  readonly currentSession = this._currentSession.asReadonly();

  readonly currentSessionLabel = computed(() => {
    const s = this._currentSession();
    if (!s) return 'No session';
    const date = new Date(s.created_at_ms);
    return `Session ${date.toLocaleDateString()} ${date.toLocaleTimeString()}`;
  });

  async initialize(): Promise<void> {
    if (!isTauriEnv()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const [sessions, current] = await Promise.all([
        invoke<SessionMeta[]>('list_sessions'),
        invoke<SessionMeta>('current_session'),
      ]);
      this._sessions.set(sessions);
      this._currentSession.set(current);
    } catch (err) {
      console.error('Failed to initialize sessions:', err);
    }
  }

  async newSession(): Promise<SessionMeta | null> {
    if (!isTauriEnv()) return null;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const meta = await invoke<SessionMeta>('new_session');
      this._currentSession.set(meta);
      this.chatService.clearLoadedState();
      await this.refreshList();
      return meta;
    } catch (err) {
      console.error('Failed to create session:', err);
      return null;
    }
  }

  async resumeSession(sessionId: string): Promise<SessionMeta | null> {
    if (!isTauriEnv()) return null;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const meta = await invoke<SessionMeta>('resume_session', { sessionId });
      this._currentSession.set(meta);
      this.chatService.clearLoadedState();
      return meta;
    } catch (err) {
      console.error('Failed to resume session:', err);
      return null;
    }
  }

  async deleteSession(sessionId: string): Promise<boolean> {
    if (!isTauriEnv()) return false;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('delete_session', { sessionId });
      await this.refreshList();
      return true;
    } catch (err) {
      console.error('Failed to delete session:', err);
      return false;
    }
  }

  private async refreshList(): Promise<void> {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const sessions = await invoke<SessionMeta[]>('list_sessions');
      this._sessions.set(sessions);
    } catch {
      // Silently fail — list is non-critical
    }
  }
}
