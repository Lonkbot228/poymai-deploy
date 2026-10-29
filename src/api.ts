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
  site_probe: string | null;
};
export type Config = {
  profiles: Profile[];
  poll_seconds: number;
  ssh_key: string;
  announce_minutes: number;
  guard_manual: boolean;
  site_poll_seconds: number;
};
export type ChangedFile = { status: string; path: string };
export type LocalState = {
  branch: string;
  head: string;
  head_message: string;
  upstream: string;
  ahead: number;
  behind: number;
  changes: ChangedFile[];
  /** Built from the changed paths; the user confirms or edits it before committing. */
  suggested_message: string;
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

/* ── Site (PoymAI app/deploy_window.py) ─────────────────────────────────── */

export type Verdict = "ok" | "warn" | "stop";
export type SlotLevel = "quiet" | "moderate" | "busy";
export type Reason = { key: string; level: Verdict | "info"; title: string; detail: string };
export type Person = {
  id: string;
  name: string;
  email: string;
  organization_name: string;
  excluded: boolean;
  online: boolean;
  tabs: number;
  last_seen_at: string | null;
  seconds_ago: number | null;
  today_seconds: number;
};
export type Hour = { at: string; hour: number; users: number; messages: number; load: number; level: SlotLevel };
export type SiteWindow = { start: string; end: string; load: number; users: number; messages: number; level: SlotLevel };
export type Notice = { starts_at: string; expires_at: string; text: string; created_at: string };
export type SiteData = {
  generated_at: string;
  verdict: { level: Verdict; title: string; reasons: Reason[] };
  signals: {
    active_users: number;
    recent_users: number;
    dashboards: number;
    visitors: number;
    messages_10m: number;
    incoming_10m: number;
    migration_imports: number;
    history_imports: number;
    scheduled_soon: number;
    payments_open: number;
    last_message_at: string | null;
  };
  people: Person[];
  server: { started_at: string; uptime_seconds: number };
  timeline: Hour[];
  windows: SiteWindow[];
  notice: Notice | null;
};
export type SiteState = { data: SiteData | null; error: string; checked_at: string };

/* ── Schedule ───────────────────────────────────────────────────────────── */

export type JobStatus = "waiting" | "announced" | "running" | "done" | "failed" | "cancelled" | "expired";
export type Job = {
  id: string;
  profile: string;
  mode: "at" | "quiet";
  run_at: string | null;
  deadline: string | null;
  allow_warn: boolean;
  force_at_deadline: boolean;
  announce_minutes: number;
  sha: string;
  message: string;
  backup_db: boolean;
  created_at: string;
  created_on: string;
  status: JobStatus;
  announced_for: string | null;
  result: string;
  finished_at: string | null;
};
export type Plan = {
  mode: "at" | "quiet";
  run_at?: string | null;
  deadline?: string | null;
  allow_warn?: boolean;
  force_at_deadline?: boolean;
  announce_minutes?: number;
  options?: DeployOptions;
};

/* ── Changes ────────────────────────────────────────────────────────────── */

export type Pending = {
  base: string;
  commits: { sha: string; subject: string; author: string; date: string; machine: string }[];
  local_files: number;
  files: string[];
  areas: { name: string; count: number }[];
  migrations: string[];
  flags: { level: "info" | "warn"; text: string }[];
};

export type Snapshot = {
  config: Config;
  statuses: ProfileStatus[];
  busy: string | null;
  machine: string;
  steps: [string, number, string][];
  jobs: Job[];
  site: Record<string, SiteState>;
};
export type DeployEvent = { profile: string; kind: string; step: string; text: string; progress: number };
export type Outcome = { ok: boolean; sha: string; status: string; text: string; code: number; seconds: number };
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
  pending: (profile: string) => invoke<Pending>("pending", { profile }),
  siteWindow: (profile: string, fresh = false) => invoke<SiteState>("site_window", { profile, fresh }),
  announce: (profile: string, minutes: number, text = "") => invoke<void>("announce", { profile, minutes, text }),
  clearNotice: (profile: string) => invoke<void>("announce", { profile, minutes: 0, text: "" }),
  deploy: (profile: string, options: DeployOptions) => invoke<Outcome>("deploy", { profile, options }),
  schedule: (profile: string, plan: Plan) => invoke<Job>("schedule_deploy", { profile, plan }),
  cancelJob: (id: string) => invoke<void>("cancel_job", { id }),
  runJobNow: (id: string) => invoke<void>("run_job_now", { id }),
  saveConfig: (config: Config) => invoke<void>("save_config", { config }),
  dismissError: () => invoke<void>("dismiss_error"),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  onDeployEvent: (cb: (e: DeployEvent) => void) => listen<DeployEvent>("deploy-event", (e) => cb(e.payload)),
  onPublishEvent: (cb: (e: DeployEvent) => void) => listen<DeployEvent>("publish-event", (e) => cb(e.payload)),
  onStatus: (cb: () => void) => listen("status-changed", () => cb()),
  onSite: (cb: (profile: string) => void) => listen<string>("site-changed", (e) => cb(e.payload)),
  onTrayDeploy: (cb: (profile: string) => void) => listen<string>("tray-deploy", (e) => cb(e.payload)),
  onTrayQuiet: (cb: (profile: string) => void) => listen<string>("tray-quiet", (e) => cb(e.payload)),
};

/* ── Formatting ─────────────────────────────────────────────────────────── */

export const short = (sha?: string | null) => (sha ? sha.slice(0, 7) : "—");

export function plural(n: number, one: string, few: string, many: string) {
  const a = Math.abs(Math.trunc(n)) % 100, b = a % 10;
  if (b === 1 && a !== 11) return one;
  if (b >= 2 && b <= 4 && (a < 10 || a >= 20)) return few;
  return many;
}

export function ago(iso?: string | null) {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const s = Math.round((Date.now() - t) / 1000);
  if (s < 45) return "только что";
  if (s < 3600) { const m = Math.round(s / 60); return `${m} ${plural(m, "минуту", "минуты", "минут")} назад`; }
  if (s < 86400) { const h = Math.round(s / 3600); return `${h} ${plural(h, "час", "часа", "часов")} назад`; }
  return when(iso);
}

const dayKey = (d: Date) => d.toDateString();

/** "сегодня в 03:00", "завтра в 03:00", "12 окт в 03:00" (local time). */
export function when(iso?: string | null) {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const time = d.toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" });
  const now = new Date();
  const tomorrow = new Date(now.getTime() + 86_400_000);
  const yesterday = new Date(now.getTime() - 86_400_000);
  if (dayKey(d) === dayKey(now)) return `сегодня в ${time}`;
  if (dayKey(d) === dayKey(tomorrow)) return `завтра в ${time}`;
  if (dayKey(d) === dayKey(yesterday)) return `вчера в ${time}`;
  return `${d.toLocaleDateString("ru-RU", { day: "numeric", month: "short" })} в ${time}`;
}

export const hhmm = (iso: string) => new Date(iso).toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" });

export function until(iso?: string | null, now = Date.now()) {
  if (!iso) return "";
  const s = Math.round((Date.parse(iso) - now) / 1000);
  if (s <= 0) return "сейчас";
  if (s < 60) return `через ${s} с`;
  const m = Math.floor(s / 60);
  if (m < 60) return `через ${m} мин`;
  const h = Math.floor(m / 60), r = m % 60;
  return `через ${h} ч${r ? ` ${r} мин` : ""}`;
}

export function duration(seconds: number) {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  if (seconds < 60) return `${Math.round(seconds)} с`;
  const m = Math.floor(seconds / 60), s = Math.round(seconds % 60);
  if (m < 60) return s ? `${m} мин ${s} с` : `${m} мин`;
  const h = Math.floor(m / 60);
  return `${h} ч ${m % 60} мин`;
}

/** <input type="datetime-local"> value ↔ ISO. */
export function toLocalInput(d: Date) {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
export const fromLocalInput = (v: string) => (v ? new Date(v).toISOString() : null);
