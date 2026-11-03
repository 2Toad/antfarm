// The JSON the Rust side sends, mirroring app/src-tauri/src/screens.rs and antfarm-view.

export type Class = "Waiting" | "Working" | "Finished" | "Idle" | "Still" | "Exited";
export type WaitingKind = "permission" | "question";
export type ResultStatus = "pending" | "ok" | "error";
export type Category = "shell" | "read" | "edit" | "search" | "web" | "spawn" | "ask" | "other";

export type Route =
  | { screen: "machine" }
  | { screen: "project"; key: string }
  | { screen: "session"; id: string }
  | { screen: "lanes"; id: string; start?: string | null; end?: string | null; playhead?: string | null }
  | { screen: "agent"; session: string; agent: string }
  | { screen: "call"; session: string; agent: string; call: string };

export interface Meters {
  sessions: number;
  agents: number;
  waiting: number;
  oldest_wait: number | null;
  working: number;
  finished: number;
  still: number;
  exited: number;
  calls_per_min: number;
  tok_per_min: number;
  spent_today: number;
  apps: string[];
}

export interface NeedsYou {
  session: string;
  name: string;
  project: string;
  kind: WaitingKind;
  request: string;
  source: string;
  age: number | null;
}

export interface AgentLine {
  id: string;
  depth: number;
  name: string;
  class: Class;
  state: string;
  detail: string;
}

export interface SessionItem {
  id: string;
  name: string;
  project: string;
  project_key: string;
  class: Class;
  state: string;
  tool: string;
  source: string;
  for_secs: number | null;
  muted: boolean;
  agents: AgentLine[];
}

export interface ProjectSummary {
  key: string;
  name: string;
  sessions: number;
  agents: number;
  waiting: number;
}

export interface Header {
  host: string;
  meters: Meters;
  needs_you: NeedsYou[];
  projects: ProjectSummary[];
  sessions: SessionItem[];
}

export interface ViewEvent {
  seq: number;
  now: string;
  header: Header;
  // Screen bodies are typed where they are used.
  body: { screen: string } & Record<string, unknown>;
}

export interface Waiting {
  kind: WaitingKind;
  agent: string;
  tool: string;
  request: string;
  reason: string | null;
  touches: string[];
  asked_at: string | null;
  after: string | null;
  age: number | null;
}

export interface CallLine {
  agent: string;
  call: string;
  agent_name: string;
  at: string | null;
  tool: string;
  target: string;
  status: ResultStatus;
  elapsed: number | null;
  waiting: boolean;
}

export interface Lane {
  label: string;
  depth: number;
  segments: [number, number, Class][];
}
