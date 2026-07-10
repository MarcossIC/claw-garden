import { Injectable, inject } from '@angular/core';
import { AgentService } from './agent.service';
import { eventBus } from '../game/event-bus';

@Injectable({ providedIn: 'root' })
export class GameBridgeService {
  private readonly agentService = inject(AgentService);

  readonly agents = this.agentService.agents;
  readonly selectedAgent = this.agentService.selectedAgent;

  selectAgent(agentId: string): void {
    this.agentService.selectAgent(agentId);
  }

  moveAgentTo(agentId: string, tileX: number, tileY: number): void {
    eventBus.emit('move-agent', { agentId, tileX, tileY });
  }
}
