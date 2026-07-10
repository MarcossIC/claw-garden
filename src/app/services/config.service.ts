import { Injectable, signal } from '@angular/core';

export type ProviderType = 'ollama' | 'anthropic' | 'openrouter';

export interface AppConfig {
  active_provider: ProviderType;
  ollama: { host: string; port: number; model: string };
  anthropic: { api_key: string; model: string };
  openrouter: { api_key: string; model: string };
  project_path: string | null;
}

export interface OllamaStatus {
  running: boolean;
  models: string[];
}

export interface OpenRouterModel {
  id: string;
  name: string;
  description?: string;
  context_length: number;
  pricing?: { prompt: string; completion: string };
  supported_parameters: string[];
}

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

const DEFAULT_CONFIG: AppConfig = {
  active_provider: 'ollama',
  ollama: { host: 'localhost', port: 11434, model: 'llama3.1' },
  anthropic: { api_key: '', model: 'claude-sonnet-4-5-20250514' },
  openrouter: { api_key: '', model: 'openrouter/free' },
  project_path: null,
};

@Injectable({ providedIn: 'root' })
export class ConfigService {
  readonly config = signal<AppConfig>(DEFAULT_CONFIG);
  readonly ollamaStatus = signal<OllamaStatus>({ running: false, models: [] });
  readonly openrouterModels = signal<OpenRouterModel[]>([]);
  readonly saving = signal(false);

  constructor() {
    this.loadConfig();
    this.checkOllama();
  }

  private async loadConfig(): Promise<void> {
    if (!isTauriEnv()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const config = await invoke<AppConfig>('get_config');
      this.config.set(config);
    } catch (err) {
      console.error('Failed to load config:', err);
    }
  }

  async checkOllama(): Promise<void> {
    if (!isTauriEnv()) {
      this.ollamaStatus.set({ running: false, models: [] });
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const status = await invoke<OllamaStatus>('check_ollama');
      this.ollamaStatus.set(status);
    } catch {
      this.ollamaStatus.set({ running: false, models: [] });
    }
  }

  async fetchOpenRouterModels(apiKey: string): Promise<void> {
    if (!isTauriEnv()) {
      this.openrouterModels.set([]);
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const models = await invoke<OpenRouterModel[]>('openrouter_list_models', { apiKey });
      this.openrouterModels.set(models);
    } catch (err) {
      console.error('Failed to fetch OpenRouter models:', err);
      this.openrouterModels.set([]);
    }
  }

  async saveProvider(
    providerType: ProviderType,
    ollama?: { host: string; port: number; model: string },
    anthropic?: { api_key: string; model: string },
    openrouter?: { api_key: string; model: string },
  ): Promise<void> {
    if (!isTauriEnv()) {
      this.config.update((c) => ({
        ...c,
        active_provider: providerType,
        ...(ollama ? { ollama } : {}),
        ...(anthropic ? { anthropic } : {}),
        ...(openrouter ? { openrouter } : {}),
      }));
      return;
    }

    this.saving.set(true);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('set_provider', {
        providerType,
        ollama: ollama ?? null,
        anthropic: anthropic ?? null,
        openrouter: openrouter ?? null,
      });
      await this.loadConfig();
    } catch (err) {
      console.error('Failed to save provider:', err);
    } finally {
      this.saving.set(false);
    }
  }

  async setProjectPath(path: string): Promise<void> {
    if (!isTauriEnv()) {
      this.config.update((c) => ({ ...c, project_path: path }));
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('set_project_path', { path });
      this.config.update((c) => ({ ...c, project_path: path }));
    } catch (err) {
      console.error('Failed to set project path:', err);
    }
  }
}
