export type AgentStateKind = 'Idle' | 'Walking' | 'Thinking' | 'Coding' | 'Reading' | 'Searching' | 'Error';

export type AgentState =
  | { kind: 'Idle' }
  | { kind: 'Walking'; target: { x: number; y: number } }
  | { kind: 'Thinking'; taskSummary: string }
  | { kind: 'Coding'; file?: string }
  | { kind: 'Reading'; source: string }
  | { kind: 'Searching'; query: string }
  | { kind: 'Error'; message: string };

export interface AgentPosition {
  x: number;
  y: number;
}

export type AgentRole = 'developer' | 'researcher' | 'reviewer' | 'planner' | 'writer';

export interface Agent {
  id: string;
  name: string;
  role: AgentRole;
  provider_id: string;
  project_path: string | null;
  custom_instructions: string | null;
  efficient_mode: boolean;
  state: AgentState;
  position: AgentPosition;
}
