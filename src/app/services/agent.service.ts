import { Injectable, signal, computed, inject } from '@angular/core';
import type { Agent, AgentRole } from '../models/agent.model';
import { eventBus } from '../game/event-bus';

function isTauriEnv(): boolean {
  return '__TAURI__' in window || '__TAURI_INTERNALS__' in window;
}

@Injectable({ providedIn: 'root' })
export class AgentService {
  readonly agents = signal<Agent[]>([]);
  readonly selectedAgent = signal<Agent | null>(null);
  readonly loading = signal(false);

  private sceneReady = false;
  private agentMap = new Map<string, Agent>();

  constructor() {
    this.loadAgents();
    this.setupEventListeners();
  }

  /** Emit agent-created for all loaded agents so Phaser creates their sprites. */
  private emitAllAgents(): void {
    for (const agent of this.agents()) {
      eventBus.emit('agent-created', {
        agentId: agent.id,
        name: agent.name,
        tileX: Math.floor(agent.position.x),
        tileY: Math.floor(agent.position.y),
      });
    }
  }

  private setupEventListeners(): void {
    // When the Phaser scene is ready, emit sprites for agents already loaded
    eventBus.on('scene-ready', () => {
      this.sceneReady = true;
      this.emitAllAgents();
    });

    eventBus.on('agent-selected', (data) => {
      this.selectedAgent.set(this.agentMap.get(data.agentId) ?? null);
    });

    eventBus.on('agent-state-changed', (data) => {
      const agent = this.agentMap.get(data.agentId);
      if (!agent) return;
      this.upsertAgent({
        ...agent,
        state: data.state,
        ...(data.position ? { position: data.position } : {}),
      });
      this.refreshSelected(data.agentId);
    });

    eventBus.on('agent-arrived', (data) => {
      const agent = this.agentMap.get(data.agentId);
      if (!agent) return;
      this.upsertAgent({
        ...agent,
        state: { kind: 'Idle' as const },
        position: { x: data.tileX, y: data.tileY },
      });
      this.refreshSelected(data.agentId);
    });
  }

  private upsertAgent(agent: Agent): void {
    this.agentMap.set(agent.id, agent);
    this.agents.set([...this.agentMap.values()]);
  }

  private removeAgentById(id: string): void {
    this.agentMap.delete(id);
    this.agents.set([...this.agentMap.values()]);
  }

  private refreshSelected(agentId: string): void {
    const selected = this.selectedAgent();
    if (selected?.id === agentId) {
      const updated = this.agentMap.get(agentId);
      if (updated) this.selectedAgent.set(updated);
    }
  }

  async loadAgents(): Promise<void> {
    if (!isTauriEnv()) return;
    this.loading.set(true);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const list = await invoke<Agent[]>('get_agents');
      this.agentMap.clear();
      list.forEach((a) => this.agentMap.set(a.id, a));
      this.agents.set(list);

      // If scene was already ready before agents loaded, emit now
      if (this.sceneReady) {
        this.emitAllAgents();
      }
    } catch (err) {
      console.error('Failed to load agents:', err);
    } finally {
      this.loading.set(false);
    }
  }

  selectAgent(agentId: string): void {
    this.selectedAgent.set(this.agentMap.get(agentId) ?? null);
  }

  async createAgent(
    name: string,
    role: AgentRole,
    providerId: string,
    projectPath?: string,
    customInstructions?: string,
  ): Promise<Agent | null> {
    if (!isTauriEnv()) return null;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const created = await invoke<Agent>('create_agent', {
        name,
        role,
        providerId,
        projectPath: projectPath || null,
        customInstructions: customInstructions || null,
      });
      this.upsertAgent(created);
      eventBus.emit('agent-created', {
        agentId: created.id,
        name: created.name,
        tileX: Math.floor(created.position.x),
        tileY: Math.floor(created.position.y),
      });
      return created;
    } catch (err) {
      console.error('Failed to create agent:', err);
      return null;
    }
  }

  async updateAgent(
    id: string,
    changes: {
      name?: string;
      role?: AgentRole;
      providerId?: string;
      projectPath?: string | null;
      customInstructions?: string | null;
      efficientMode?: boolean;
    },
  ): Promise<Agent | null> {
    if (!isTauriEnv()) return null;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const updated = await invoke<Agent>('update_agent', {
        id,
        name: changes.name ?? null,
        role: changes.role ?? null,
        providerId: changes.providerId ?? null,
        projectPath: changes.projectPath !== undefined ? changes.projectPath : null,
        customInstructions: changes.customInstructions !== undefined ? changes.customInstructions : null,
        efficientMode: changes.efficientMode ?? null,
      });
      this.upsertAgent(updated);
      if (this.selectedAgent()?.id === id) {
        this.selectedAgent.set(updated);
      }
      return updated;
    } catch (err) {
      console.error('Failed to update agent:', err);
      return null;
    }
  }

  async deleteAgent(id: string): Promise<boolean> {
    if (!isTauriEnv()) return false;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('delete_agent', { id });
      this.removeAgentById(id);
      if (this.selectedAgent()?.id === id) {
        this.selectedAgent.set(null);
      }
      eventBus.emit('agent-deleted', { agentId: id });
      return true;
    } catch (err) {
      console.error('Failed to delete agent:', err);
      return false;
    }
  }
}
