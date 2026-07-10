import { Injectable, signal } from '@angular/core';

export interface OAuthStatus {
  logged_in: boolean;
  expired: boolean;
  has_refresh_token: boolean;
}

export interface AuthStatus {
  anthropic_source: string; // 'env' | 'oauth' | 'config' | 'none'
  openrouter_source: string;
}

export interface OAuthLoginResult {
  success: boolean;
  message: string;
}

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

@Injectable({ providedIn: 'root' })
export class AuthService {
  readonly oauthStatus = signal<OAuthStatus>({ logged_in: false, expired: false, has_refresh_token: false });
  readonly authStatus = signal<AuthStatus>({ anthropic_source: 'none', openrouter_source: 'none' });
  readonly loading = signal(false);

  constructor() {
    this.refresh();
  }

  /** Fetch both OAuth status and auth sources from backend. */
  async refresh(): Promise<void> {
    if (!isTauriEnv()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const [oauth, auth] = await Promise.all([
        invoke<OAuthStatus>('oauth_status'),
        invoke<AuthStatus>('get_auth_status'),
      ]);
      this.oauthStatus.set(oauth);
      this.authStatus.set(auth);
    } catch (err) {
      console.error('Failed to fetch auth status:', err);
    }
  }

  /** Start the full OAuth PKCE login flow (opens browser). */
  async login(): Promise<OAuthLoginResult> {
    if (!isTauriEnv()) return { success: false, message: 'Not in Tauri environment' };
    this.loading.set(true);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<OAuthLoginResult>('oauth_login');
      await this.refresh();
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      return { success: false, message };
    } finally {
      this.loading.set(false);
    }
  }

  /** Logout — clear stored OAuth credentials. */
  async logout(): Promise<void> {
    if (!isTauriEnv()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('oauth_logout');
      await this.refresh();
    } catch (err) {
      console.error('Logout failed:', err);
    }
  }

  /** Manually refresh the OAuth token. */
  async refreshToken(): Promise<OAuthLoginResult> {
    if (!isTauriEnv()) return { success: false, message: 'Not in Tauri environment' };
    this.loading.set(true);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<OAuthLoginResult>('oauth_refresh');
      await this.refresh();
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      return { success: false, message };
    } finally {
      this.loading.set(false);
    }
  }
}
