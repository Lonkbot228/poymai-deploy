import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Route = { name: string; user: string; host: string; port: number; jump?: string | null };
export type Profile = {
  id: string;
  name: string;
  repo_path: string;
  branch: string;
  remote_dir: string;
  routes: Route[];
  checks: string[];
  auto_pull: boolean;
};
export type Config = { profiles: Profile[]; poll_seconds: number; ssh_key: string };
export type ChangedFile = { status: string; path: string };
export type LocalState = {
  branch: string;
  head: string;
  head_message: string;
  upstream: string;
  ahead: number;
  behind: number;
  changes: ChangedFile[];
};
export type ProfileStatus = {
  id: string;
  name: string;
  local: LocalState | null;
  github: string;
  production: string;
  error: string;
  checked_at: string;
  incoming: number;
  overlap: string[];
};
export type Snapshot = {
  config: Config;
  statuses: ProfileStatus[];
  busy: string | null;
  machine: string;
  steps: [string, number, string][];
};
export type DeployEvent = { profile: string; kind: string; step: string; text: string; progress: number };
export type Outcome = { ok: boolean; sha: string; status: string; text: string; code: number };
export type Release = {
  sha: string;
  prev: string;
  status: string;
  actor: string;
  machine: string;
  started: string;
  finished: string;
  message: string;
};
export type ServerStatus = {
  head: string;
  drift: number;
  busy: string;
  route: string;
  current: Release | null;
};
export type DeployOptions = {
  message?: string;
  sha?: string;
  dry_run?: boolean;
  accept_drift?: boolean;
  backup_db?: boolean;
};

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  refresh: (profile?: string) => invoke<void>("refresh", { profile }),
  serverStatus: (profile: string) => invoke<ServerStatus>("server_status", { profile }),
  releases: (profile: string) => invoke<Release[]>("releases", { profile }),
  pullUpdates: (profile: string) => invoke<string>("pull_updates", { profile }),
  deploy: (profile: string, options: DeployOptions) => invoke<Outcome>("deploy", { profile, options }),
  saveConfig: (config: Config) => invoke<void>("save_config", { config }),
  dismissError: () => invoke<void>("dismiss_error"),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  onDeployEvent: (cb: (e: DeployEvent) => void) => listen<DeployEvent>("deploy-event", (e) => cb(e.payload)),
  onStatus: (cb: () => void) => listen("status-changed", () => cb()),
  onTrayDeploy: (cb: (profile: string) => void) => listen<string>("tray-deploy", (e) => cb(e.payload)),
};

export const short = (sha?: string | null) => (sha ? sha.slice(0, 7) : "—");

export function ago(iso: string) {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const s = Math.round((Date.now() - t) / 1000);
  if (s < 60) return "только что";
  if (s < 3600) return `${Math.round(s / 60)} мин назад`;
  if (s < 86400) return `${Math.round(s / 3600)} ч назад`;
  return new Date(t).toLocaleString("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}
