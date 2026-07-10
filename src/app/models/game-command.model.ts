import type { AgentState } from './agent.model';

export type GameCommand =
  | { type: 'SPAWN_AGENT'; agentId: string; name: string; tileX: number; tileY: number }
  | { type: 'REMOVE_AGENT'; agentId: string }
  | { type: 'SET_AGENT_STATE'; agentId: string; state: AgentState }
  | { type: 'MOVE_AGENT'; agentId: string; tileX: number; tileY: number };
