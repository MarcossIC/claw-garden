import { Component, ChangeDetectionStrategy, inject, signal, computed } from '@angular/core';
import { DecimalPipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { ConfigService, type ProviderType } from '../../services/config.service';
import { EmbedderService } from '../../services/embedder.service';

@Component({
  selector: 'app-config-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './config-panel.component.html',
  styleUrl: './config-panel.component.css',
  imports: [FormsModule, DecimalPipe],
})
export class ConfigPanelComponent {
  readonly configService = inject(ConfigService);
  readonly embedderService = inject(EmbedderService);

  /**
   * Integer percent 0..100 for the embedder download progress bar.
   * Computed (not a template arrow fn) so the template stays declarative
   * and OnPush change detection re-evaluates on signal change.
   */
  protected readonly embedderProgressPercent = computed(() => {
    const s = this.embedderService.status();
    if (s.phase !== 'downloading') return 0;
    const total = s.bytesTotal;
    if (!total || total <= 0) return 0;
    const downloaded = s.bytesDownloaded ?? 0;
    const pct = Math.round((downloaded * 100) / total);
    if (pct < 0) return 0;
    if (pct > 100) return 100;
    return pct;
  });
  readonly saveStatus = signal<'idle' | 'saving' | 'saved' | 'error'>('idle');
  readonly saveMessage = signal('');

  readonly providerType = signal<ProviderType>('ollama');
  readonly ollamaHost = signal('localhost');
  readonly ollamaPort = signal(11434);
  readonly ollamaModel = signal('llama3.1');
  readonly anthropicApiKey = signal('');
  readonly anthropicModel = signal('claude-sonnet-4-5-20250514');
  readonly openrouterApiKey = signal('');
  readonly openrouterModel = signal('openrouter/free');
  readonly openrouterSearch = signal('');
  readonly openrouterFreeOnly = signal(false);
  readonly openrouterToolsOnly = signal(false);
  readonly projectPath = signal('');

  readonly ollamaRunning = computed(() => this.configService.ollamaStatus().running);
  readonly ollamaModels = computed(() => this.configService.ollamaStatus().models);

  readonly filteredOpenRouterModels = computed(() => {
    let models = this.configService.openrouterModels();
    const search = this.openrouterSearch().toLowerCase();
    const freeOnly = this.openrouterFreeOnly();
    const toolsOnly = this.openrouterToolsOnly();

    if (search) {
      models = models.filter(
        (m) => m.name.toLowerCase().includes(search) || m.id.toLowerCase().includes(search),
      );
    }
    if (freeOnly) {
      models = models.filter(
        (m) => m.pricing?.prompt === '0' && m.pricing?.completion === '0',
      );
    }
    if (toolsOnly) {
      models = models.filter((m) => m.supported_parameters.includes('tools'));
    }
    return models.slice(0, 50); // Limit display to 50
  });

  constructor() {
    const config = this.configService.config();
    this.providerType.set(config.active_provider);
    this.ollamaHost.set(config.ollama.host);
    this.ollamaPort.set(config.ollama.port);
    this.ollamaModel.set(config.ollama.model);
    this.anthropicApiKey.set(config.anthropic.api_key);
    this.anthropicModel.set(config.anthropic.model);
    this.openrouterApiKey.set(config.openrouter.api_key);
    this.openrouterModel.set(config.openrouter.model);
    this.projectPath.set(config.project_path ?? '');
  }

  async saveProvider(): Promise<void> {
    this.saveStatus.set('saving');
    try {
      const type = this.providerType();
      await this.configService.saveProvider(
        type,
        type === 'ollama'
          ? { host: this.ollamaHost(), port: this.ollamaPort(), model: this.ollamaModel() }
          : undefined,
        type === 'anthropic'
          ? { api_key: this.anthropicApiKey(), model: this.anthropicModel() }
          : undefined,
        type === 'openrouter'
          ? { api_key: this.openrouterApiKey(), model: this.openrouterModel() }
          : undefined,
      );
      this.saveStatus.set('saved');
      this.saveMessage.set(`Provider "${type}" saved`);
      setTimeout(() => this.saveStatus.set('idle'), 3000);
    } catch {
      this.saveStatus.set('error');
      this.saveMessage.set('Failed to save provider');
      setTimeout(() => this.saveStatus.set('idle'), 4000);
    }
  }

  async saveProjectPath(): Promise<void> {
    const path = this.projectPath();
    if (path.trim()) {
      await this.configService.setProjectPath(path.trim());
    }
  }

  async refreshOllama(): Promise<void> {
    await this.configService.checkOllama();
  }

  async loadOpenRouterModels(): Promise<void> {
    const key = this.openrouterApiKey();
    if (key.trim()) {
      await this.configService.fetchOpenRouterModels(key.trim());
    }
  }

  selectOpenRouterModel(modelId: string): void {
    this.openrouterModel.set(modelId);
  }

  isFreeModel(model: { pricing?: { prompt: string; completion: string } }): boolean {
    return model.pricing?.prompt === '0' && model.pricing?.completion === '0';
  }
}
