import type { AgentState } from './agent.model';

export interface GameEventMap {
  'agent-selected': { agentId: string };
  'agent-state-changed': {
    agentId: string;
    state: AgentState;
    position?: { x: number; y: number };
  };
  'tile-clicked': { tileX: number; tileY: number; worldX: number; worldY: number };
  'agent-arrived': { agentId: string; tileX: number; tileY: number };
  'move-agent': { agentId: string; tileX: number; tileY: number };
  'agent-created': { agentId: string; name: string; tileX: number; tileY: number };
  'agent-deleted': { agentId: string };
  'scene-ready': void;
}
