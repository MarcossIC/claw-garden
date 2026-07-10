import { Injectable, signal } from '@angular/core';
import type { ProviderConfig, ProviderType } from '../models/provider.model';

export interface OpenRouterModel {
  id: string;
  name: string;
  context_length: number;
  pricing?: { prompt: string; completion: string };
  supported_parameters: string[];
}

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

@Injectable({ providedIn: 'root' })
export class ProviderService {
  readonly providers = signal<ProviderConfig[]>([]);
  readonly loading = signal(false);
  readonly openrouterModels = signal<OpenRouterModel[]>([]);
  readonly loadingModels = signal(false);

  constructor() {
    this.loadProviders();
  }

  async loadProviders(): Promise<void> {
    if (!isTauriEnv()) return;
    this.loading.set(true);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const list = await invoke<ProviderConfig[]>('list_providers');
      this.providers.set(list);
    } catch (err) {
      console.error('Failed to load providers:', err);
    } finally {
      this.loading.set(false);
    }
  }

  async addProvider(
    name: string,
    providerType: ProviderType,
    ollama?: { host: string; port: number; model: string },
    anthropic?: { api_key: string; model: string },
    openrouter?: { api_key: string; model: string },
  ): Promise<ProviderConfig | null> {
    if (!isTauriEnv()) return null;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const created = await invoke<ProviderConfig>('add_provider', {
        name,
        providerType,
        ollama: ollama ?? null,
        anthropic: anthropic ?? null,
        openrouter: openrouter ?? null,
      });
      this.providers.update((list) => [...list, created]);
      return created;
    } catch (err) {
      console.error('Failed to add provider:', err);
      return null;
    }
  }

  async updateProvider(
    id: string,
    name: string,
    providerType: ProviderType,
    ollama?: { host: string; port: number; model: string },
    anthropic?: { api_key: string; model: string },
    openrouter?: { api_key: string; model: string },
  ): Promise<ProviderConfig | null> {
    if (!isTauriEnv()) return null;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const updated = await invoke<ProviderConfig>('update_provider', {
        id,
        name,
        providerType,
        ollama: ollama ?? null,
        anthropic: anthropic ?? null,
        openrouter: openrouter ?? null,
      });
      this.providers.update((list) => list.map((p) => (p.id === id ? updated : p)));
      return updated;
    } catch (err) {
      console.error('Failed to update provider:', err);
      return null;
    }
  }

  async deleteProvider(id: string): Promise<boolean> {
    if (!isTauriEnv()) return false;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('delete_provider', { id });
      this.providers.update((list) => list.filter((p) => p.id !== id));
      return true;
    } catch (err) {
      console.error('Failed to delete provider:', err);
      return false;
    }
  }

  async loadOpenRouterModels(apiKey: string): Promise<void> {
    if (!isTauriEnv() || !apiKey.trim()) return;
    this.loadingModels.set(true);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const models = await invoke<OpenRouterModel[]>('openrouter_list_models', { apiKey });
      this.openrouterModels.set(models);
    } catch (err) {
      console.error('Failed to fetch OpenRouter models:', err);
      this.openrouterModels.set([]);
    } finally {
      this.loadingModels.set(false);
    }
  }

  isFreeModel(model: OpenRouterModel): boolean {
    return model.pricing?.prompt === '0' && model.pricing?.completion === '0';
  }
}
