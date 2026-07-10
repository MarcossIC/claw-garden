export type ProviderType = 'ollama' | 'anthropic' | 'openrouter';

export interface ProviderConfig {
  id: string;
  name: string;
  provider_type: ProviderType;
  ollama?: { host: string; port: number; model: string };
  anthropic?: { api_key: string; model: string; base_url?: string };
  openrouter?: { api_key: string; model: string; base_url?: string };
}
