import {
  Component,
  ChangeDetectionStrategy,
  inject,
  signal,
  computed,
  effect,
} from '@angular/core';
import { FormsModule } from '@angular/forms';
import { AgentService } from '../../services/agent.service';
import { AuthService } from '../../services/auth.service';
import { ProviderService } from '../../services/provider.service';
import { WorkspaceConfigService } from '../../services/workspace-config.service';
import { ChatPanelComponent } from '../chat-panel/chat-panel.component';
import type { ProviderConfig, ProviderType } from '../../models/provider.model';
import type { AgentRole } from '../../models/agent.model';

type TabId = 'providers' | 'agents' | 'chat';

@Component({
  selector: 'app-sidebar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './sidebar.component.html',
  styleUrl: './sidebar.component.css',
  imports: [FormsModule, ChatPanelComponent],
})
export class SidebarComponent {
  readonly agentService = inject(AgentService);
  readonly authService = inject(AuthService);
  readonly providerService = inject(ProviderService);
  readonly workspaceConfigService = inject(WorkspaceConfigService);
  readonly activeTab = signal<TabId>('providers');

  // ── Provider form state ──
  readonly showProviderForm = signal(false);
  readonly editingProviderId = signal<string | null>(null);
  readonly providerFormName = signal('');
  readonly providerFormType = signal<ProviderType>('openrouter');
  readonly providerFormOllamaHost = signal('localhost');
  readonly providerFormOllamaPort = signal(11434);
  readonly providerFormOllamaModel = signal('llama3.1');
  readonly providerFormAnthropicKey = signal('');
  readonly providerFormAnthropicModel = signal('claude-sonnet-4-5-20250514');
  readonly providerFormOpenrouterKey = signal('');
  readonly providerFormOpenrouterModel = signal('openrouter/auto');
  readonly openrouterSearch = signal('');
  readonly openrouterFreeOnly = signal(false);

  readonly filteredModels = computed(() => {
    let models = this.providerService.openrouterModels();
    const search = this.openrouterSearch().toLowerCase();
    const freeOnly = this.openrouterFreeOnly();

    if (search) {
      models = models.filter(
        (m) => m.name.toLowerCase().includes(search) || m.id.toLowerCase().includes(search),
      );
    }
    if (freeOnly) {
      models = models.filter((m) => this.providerService.isFreeModel(m));
    }
    return models.slice(0, 40);
  });

  // ── Agent form state ──
  readonly showAgentForm = signal(false);
  readonly editingAgentId = signal<string | null>(null);
  readonly agentFormName = signal('');
  readonly agentFormRole = signal<AgentRole>('developer');
  readonly agentFormProviderId = signal('');
  readonly agentFormProjectPath = signal('');
  readonly agentFormInstructions = signal('');

  // ── Workspace config form state ──
  readonly workspaceExpanded = signal(false);
  readonly wsProviderId = signal('');

  // ── Feedback ──
  readonly feedback = signal('');

  private previousAgentId: string | null = null;

  constructor() {
    effect(() => {
      const agent = this.agentService.selectedAgent();
      if (agent && agent.id !== this.previousAgentId) {
        this.previousAgentId = agent.id;
        this.activeTab.set('chat');
      }
    });

    // Trigger a workspace config load whenever the agent form's project path changes.
    // No .then() here — a separate effect below syncs wsProviderId reactively.
    effect(() => {
      const path = this.agentFormProjectPath();
      if (path.trim()) {
        this.workspaceConfigService.loadForPath(path.trim());
      } else {
        this.workspaceExpanded.set(false);
        this.workspaceConfigService.reset();
      }
    });

    // Sync wsProviderId whenever a new workspace config finishes loading.
    // Reacts to loadedPath (set atomically with config after each successful load),
    // so it never races with an in-flight request.
    effect(() => {
      const loadedPath = this.workspaceConfigService.loadedPath();
      if (loadedPath) {
        this.wsProviderId.set(this.workspaceConfigService.config().provider_id ?? '');
      } else {
        this.wsProviderId.set('');
      }
    });
  }

  setTab(tab: TabId): void {
    this.activeTab.set(tab);
  }

  // ── Provider CRUD ──

  openAddProvider(): void {
    this.editingProviderId.set(null);
    this.providerFormName.set('');
    this.providerFormType.set('openrouter');
    this.providerFormOllamaHost.set('localhost');
    this.providerFormOllamaPort.set(11434);
    this.providerFormOllamaModel.set('llama3.1');
    this.providerFormAnthropicKey.set('');
    this.providerFormAnthropicModel.set('claude-sonnet-4-5-20250514');
    this.providerFormOpenrouterKey.set('');
    this.providerFormOpenrouterModel.set('openrouter/auto');
    this.showProviderForm.set(true);
  }

  openEditProvider(provider: ProviderConfig): void {
    this.editingProviderId.set(provider.id);
    this.providerFormName.set(provider.name);
    this.providerFormType.set(provider.provider_type);
    if (provider.ollama) {
      this.providerFormOllamaHost.set(provider.ollama.host);
      this.providerFormOllamaPort.set(provider.ollama.port);
      this.providerFormOllamaModel.set(provider.ollama.model);
    }
    if (provider.anthropic) {
      this.providerFormAnthropicKey.set(provider.anthropic.api_key);
      this.providerFormAnthropicModel.set(provider.anthropic.model);
    }
    if (provider.openrouter) {
      this.providerFormOpenrouterKey.set(provider.openrouter.api_key);
      this.providerFormOpenrouterModel.set(provider.openrouter.model);
    }
    this.showProviderForm.set(true);
  }

  async saveProvider(): Promise<void> {
    const name = this.providerFormName().trim();
    if (!name) return;

    const type = this.providerFormType();
    const ollama =
      type === 'ollama'
        ? { host: this.providerFormOllamaHost(), port: this.providerFormOllamaPort(), model: this.providerFormOllamaModel() }
        : undefined;
    const anthropic =
      type === 'anthropic'
        ? { api_key: this.providerFormAnthropicKey(), model: this.providerFormAnthropicModel() }
        : undefined;
    const openrouter =
      type === 'openrouter'
        ? { api_key: this.providerFormOpenrouterKey(), model: this.providerFormOpenrouterModel() }
        : undefined;

    const editId = this.editingProviderId();
    if (editId) {
      await this.providerService.updateProvider(editId, name, type, ollama, anthropic, openrouter);
      this.showFeedback('Provider updated');
    } else {
      await this.providerService.addProvider(name, type, ollama, anthropic, openrouter);
      this.showFeedback('Provider created');
    }
    this.showProviderForm.set(false);
  }

  async removeProvider(id: string): Promise<void> {
    const ok = await this.providerService.deleteProvider(id);
    if (ok) {
      this.showFeedback('Provider deleted');
    } else {
      this.showFeedback('Cannot delete: provider in use');
    }
  }

  async loadOpenRouterModels(): Promise<void> {
    const key = this.providerFormOpenrouterKey().trim();
    if (key) {
      await this.providerService.loadOpenRouterModels(key);
    }
  }

  selectOpenRouterModel(modelId: string): void {
    this.providerFormOpenrouterModel.set(modelId);
  }

  cancelProviderForm(): void {
    this.showProviderForm.set(false);
  }

  toggleWorkspaceConfig(): void {
    this.workspaceExpanded.update((v) => !v);
  }

  async saveWorkspaceConfig(): Promise<void> {
    const path = this.agentFormProjectPath().trim();
    if (!path) return;
    try {
      await this.workspaceConfigService.saveForPath(path, {
        provider_id: this.wsProviderId() || null,
      });
      this.showFeedback('Workspace config saved');
    } catch {
      this.showFeedback('Failed to save workspace config');
    }
  }

  async browseProjectFolder(): Promise<void> {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true, multiple: false, title: 'Select project folder' });
      if (selected) {
        this.agentFormProjectPath.set(selected as string);
        // Scroll input to show end of path
        requestAnimationFrame(() => {
          const input = document.querySelector('.path-input') as HTMLInputElement;
          if (input) {
            input.focus();
            input.setSelectionRange(input.value.length, input.value.length);
            input.scrollLeft = input.scrollWidth;
          }
        });
      }
    } catch {
      // Not in Tauri or user cancelled
    }
  }

  // ── Agent CRUD ──

  openAddAgent(): void {
    this.editingAgentId.set(null);
    this.agentFormName.set('');
    this.agentFormRole.set('developer');
    this.agentFormProviderId.set(this.providerService.providers()[0]?.id ?? '');
    this.agentFormProjectPath.set('');
    this.agentFormInstructions.set('');
    this.showAgentForm.set(true);
  }

  openEditAgent(agentId: string): void {
    const agent = this.agentService.agents().find((a) => a.id === agentId);
    if (!agent) return;
    this.editingAgentId.set(agent.id);
    this.agentFormName.set(agent.name);
    this.agentFormRole.set(agent.role);
    this.agentFormProviderId.set(agent.provider_id);
    this.agentFormProjectPath.set(agent.project_path ?? '');
    this.agentFormInstructions.set(agent.custom_instructions ?? '');
    this.showAgentForm.set(true);
  }

  async saveAgent(): Promise<void> {
    const name = this.agentFormName().trim();
    const providerId = this.agentFormProviderId();
    if (!name || !providerId) return;

    const editId = this.editingAgentId();
    if (editId) {
      await this.agentService.updateAgent(editId, {
        name,
        role: this.agentFormRole(),
        providerId,
        projectPath: this.agentFormProjectPath().trim() || null,
        customInstructions: this.agentFormInstructions().trim() || null,
      });
      this.showFeedback('Agent updated');
    } else {
      await this.agentService.createAgent(
        name,
        this.agentFormRole(),
        providerId,
        this.agentFormProjectPath().trim() || undefined,
        this.agentFormInstructions().trim() || undefined,
      );
      this.showFeedback('Agent created');
    }
    this.showAgentForm.set(false);
  }

  async removeAgent(id: string): Promise<void> {
    await this.agentService.deleteAgent(id);
    this.showFeedback('Agent deleted');
  }

  cancelAgentForm(): void {
    this.showAgentForm.set(false);
  }

  selectAgent(id: string): void {
    this.agentService.selectAgent(id);
    this.activeTab.set('chat');
  }

  getProviderName(providerId: string): string {
    return this.providerService.providers().find((p) => p.id === providerId)?.name ?? 'Unknown';
  }

  // ── OAuth ──

  async oauthLogin(): Promise<void> {
    const result = await this.authService.login();
    this.showFeedback(result.message);
  }

  async oauthLogout(): Promise<void> {
    await this.authService.logout();
    this.showFeedback('Logged out from Anthropic');
  }

  private showFeedback(msg: string): void {
    this.feedback.set(msg);
    setTimeout(() => this.feedback.set(''), 3000);
  }
}
