import { useCallback, useEffect, useMemo, useState } from "react";
import { api, short, when, type DeployOptions, type Pending, type ProfileStatus, type Release, type ServerStatus, type Snapshot } from "./api";
import { Composer, IncomingCard, PendingLine, RunCard, SyncLine, type Run } from "./components/deploy";
import logo from "./assets/logo.png";
import { IconCalendar, IconHistory, IconHome, IconRefresh, IconSettings } from "./components/icons";
import { AnnounceSheet, GuardSheet } from "./components/modals";
import { HistoryPage, SettingsPage } from "./components/pages";
import { PlanPage, ScheduleSheet, type PresetId } from "./components/plan";
import { StatusHero } from "./components/site";
import { WindowControls } from "./components/window-controls";

type Page = "overview" | "plan" | "history" | "settings";
type ScheduleRequest = { options: DeployOptions; preset?: PresetId };

const SUCCESS_CLOSE_MS = 5 * 60_000;

const PAGES: { id: Page; label: string; icon: typeof IconHome }[] = [
  { id: "overview", label: "Деплой", icon: IconHome },
  { id: "plan", label: "План", icon: IconCalendar },
  { id: "history", label: "История", icon: IconHistory },
  { id: "settings", label: "", icon: IconSettings },
];

/** The project dot answers one question: is there anything to deploy or pull? */
function changeState(st: ProfileStatus | undefined, deploying: boolean): { tone: string; hint: string } {
  if (deploying) return { tone: "busy", hint: "Идёт деплой" };
  if (!st) return { tone: "", hint: "Проверяю…" };
  if (st.error) return { tone: "bad", hint: st.error };
  const local = st.local;
  if (local?.changes.length) return { tone: "warn", hint: `${local.changes.length} изменённых файлов` };
  if (local?.ahead) return { tone: "warn", hint: `${local.ahead} коммитов не отправлено` };
  if (st.incoming) return { tone: "warn", hint: `${st.incoming} новых коммитов на GitHub` };
  if (st.github && st.production && st.github !== st.production) return { tone: "warn", hint: "На сервере не последняя версия" };
  return { tone: "ok", hint: "Изменений нет" };
}

export default function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [active, setActive] = useState("main");
  const [page, setPage] = useState<Page>("overview");
  const [run, setRun] = useState<Run | null>(null);
  const [server, setServer] = useState<Record<string, ServerStatus | string>>({});
  const [pending, setPending] = useState<Record<string, Pending | string>>({});
  const [refreshing, setRefreshing] = useState(false);
  const [schedule, setSchedule] = useState<ScheduleRequest | null>(null);
  const [guard, setGuard] = useState<DeployOptions | null>(null);
  const [announcing, setAnnouncing] = useState(false);
  const [toast, setToast] = useState("");

  const load = useCallback(() => api.snapshot().then(setSnap), []);
  const flash = useCallback((text: string) => { setToast(text); window.setTimeout(() => setToast(""), 3500); }, []);
  const loadServer = useCallback((id: string) => {
    api.serverStatus(id).then((s) => setServer((m) => ({ ...m, [id]: s }))).catch((e) => setServer((m) => ({ ...m, [id]: String(e) })));
  }, []);
  const loadPending = useCallback((id: string) => {
    api.pending(id).then((p) => setPending((m) => ({ ...m, [id]: p }))).catch((e) => setPending((m) => ({ ...m, [id]: String(e) })));
  }, []);

  const startDeploy = useCallback(async (profile: string, options: DeployOptions) => {
    setActive(profile);
    setPage("overview");
    setRun({ profile, progress: 0, step: "", log: [], started: Date.now() });
    try {
      const outcome = await api.deploy(profile, options);
      setRun((r) => r && { ...r, outcome, progress: outcome.ok ? 100 : r.progress });
    } catch (e) {
      setRun((r) => r && { ...r, error: String(e) });
    }
    load();
    loadServer(profile);
    loadPending(profile);
  }, [load, loadServer, loadPending]);

  useEffect(() => {
    load();
    const subs = [
      api.onStatus(load),
      api.onSite(() => load()),
      api.onDeployEvent((e) =>
        setRun((r) => {
          // A scheduled deploy starts without a click: open its run card.
          const base = r && r.profile === e.profile && !r.outcome && !r.error ? r : { profile: e.profile, progress: 0, step: "", log: [], started: Date.now() };
          return {
            ...base,
            progress: Math.max(base.progress, e.progress),
            step: e.kind === "step" ? e.text : base.step,
            log: [...base.log, e].slice(-800),
            outcome: e.kind === "done" ? { ok: true, sha: "", status: "success", text: e.text, code: 0, seconds: Math.round((Date.now() - base.started) / 1000) } : base.outcome,
            error: e.kind === "error" ? e.text : base.error,
          };
        }),
      ),
      api.onTrayDeploy((p) => requestDeploy(p, {})),
      api.onTrayQuiet((p) => { setActive(p); setSchedule({ options: {}, preset: "quiet" }); }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, [load]); // eslint-disable-line react-hooks/exhaustive-deps

  // A success needs no attention: it closes itself after 5 minutes. Errors stay until closed.
  const succeeded = run?.outcome?.ok && !run.error ? run.started : null;
  useEffect(() => {
    if (succeeded === null) return;
    const timer = window.setTimeout(() => setRun((r) => (r && r.started === succeeded && r.outcome?.ok && !r.error ? null : r)), SUCCESS_CLOSE_MS);
    return () => window.clearTimeout(timer);
  }, [succeeded]);

  useEffect(() => {
    if (!snap) return;
    loadServer(active);
    loadPending(active);
    if (!snap.site[active]?.data) void api.siteWindow(active).then(load).catch(() => {});
  }, [active, !!snap]); // eslint-disable-line react-hooks/exhaustive-deps

  const site = snap?.site[active];
  const profile = snap?.config.profiles.find((p) => p.id === active);
  const hasSite = !!profile?.site_probe;
  const activeJobs = useMemo(() => (snap?.jobs ?? []).filter((j) => ["waiting", "announced", "running"].includes(j.status)), [snap]);

  function requestDeploy(profileId: string, options: DeployOptions) {
    const level = snap?.site[profileId]?.data?.verdict.level;
    if (snap?.config.guard_manual && !options.dry_run && !options.sha && level && level !== "ok") {
      setActive(profileId);
      setGuard(options);
      return;
    }
    void startDeploy(profileId, options);
  }

  const refresh = async () => {
    setRefreshing(true);
    api.refresh(active);
    loadServer(active);
    loadPending(active);
    try { if (hasSite) await api.siteWindow(active, true); } catch { /* shown in the hero */ }
    await load();
    setRefreshing(false);
  };

  if (!snap || !profile) return <div className="boot" data-tauri-drag-region><img className="logo" src={logo} alt="PoymAI" draggable={false} /></div>;

  const status = snap.statuses.find((s) => s.id === active);
  const busy = snap.busy !== null || (run !== null && !run.outcome && !run.error);
  const names = Object.fromEntries(snap.config.profiles.map((p) => [p.id, p.name]));
  const srv = server[active];
  const nextJob = activeJobs.find((j) => j.profile === active);

  return (
    <div className="app">
      <header className="top" data-tauri-drag-region>
        <img className="logo" src={logo} alt="PoymAI" draggable={false} data-tauri-drag-region />
        <div className="projects" data-tauri-drag-region>
          {snap.config.profiles.map((p) => {
            const { tone, hint } = changeState(snap.statuses.find((s) => s.id === p.id), snap.busy === p.id);
            return <button key={p.id} className={p.id === active ? "on" : ""} onClick={() => setActive(p.id)} title={hint}><span className={`dot ${tone}`} />{p.name}</button>;
          })}
        </div>
        <button className="icon-btn" onClick={refresh} title="Обновить"><IconRefresh size={14} className={refreshing ? "spin" : ""} /></button>
        <span className="top-sep" data-tauri-drag-region />
        <WindowControls />
      </header>

      <nav className="tabs">
        {PAGES.map((p) => (
          <button key={p.id} className={`${page === p.id ? "on" : ""}${p.label ? "" : " icon-only"}`} onClick={() => setPage(p.id)} title={p.label || "Настройки"}>
            <p.icon size={14} />{p.label}
            {p.id === "plan" && activeJobs.length > 0 && <span className="count">{activeJobs.length}</span>}
          </button>
        ))}
      </nav>

      <main className="content">
        {page === "overview" && (
          <div className="stack">
            {hasSite && <StatusHero state={site} onIdeal={() => setSchedule({ options: {}, preset: "best" })} />}
            {nextJob && (
              <button className="next-job" onClick={() => setPage("plan")}>
                <IconCalendar size={14} /><span className="grow">Запланирован деплой {nextJob.announced_for ? when(nextJob.announced_for) : nextJob.mode === "at" ? when(nextJob.run_at) : "когда никого нет"}</span><code>{short(nextJob.sha)}</code>
              </button>
            )}
            {run && run.profile === active
              ? <RunCard run={run} onClose={() => { setRun(null); api.dismissError(); }} onRetry={(o) => startDeploy(active, o)} />
              : <Composer status={status} busy={busy} onDeploy={(o) => requestDeploy(active, o)} onSchedule={(o) => setSchedule({ options: o })} />}
            <SyncLine status={status} server={srv} />
            {status && status.incoming > 0 && <IncomingCard status={status} onDone={load} />}
            <PendingLine pending={typeof pending[active] === "object" ? pending[active] as Pending : undefined} error={typeof pending[active] === "string" ? pending[active] as string : undefined} />
          </div>
        )}
        {page === "plan" && (
          <PlanPage site={site} hasSite={hasSite} jobs={snap.jobs} names={names} busy={busy} announce={snap.config.announce_minutes}
            onPick={(preset) => setSchedule({ options: {}, preset })}
            onBanner={() => setAnnouncing(true)}
            onClearBanner={async () => { try { await api.clearNotice(active); flash("Баннер убран"); } catch (e) { flash(String(e)); } load(); }} />
        )}
        {page === "history" && (
          <HistoryPage profile={active} current={typeof srv === "object" ? srv.head : ""} busy={busy}
            onRollback={(r: Release) => { if (confirm(`Откатить сервер на ${short(r.sha)} «${r.message}»?`)) void startDeploy(active, { sha: r.sha }); }} />
        )}
        {page === "settings" && (
          <SettingsPage config={snap.config} onSaved={load} onTestSite={async (id) => {
            try {
              const s = await api.siteWindow(id, true);
              await load();
              return s.data ? `Есть связь: «${s.data.verdict.title}»` : s.error || "Нет ответа";
            } catch (e) { return String(e); }
          }} />
        )}
      </main>

      {schedule && (
        <ScheduleSheet profile={active} name={profile.name} site={site?.data} initial={schedule.options} preset={schedule.preset}
          announce={snap.config.announce_minutes} onClose={() => setSchedule(null)}
          onDone={(t) => { setSchedule(null); flash(t); setPage("plan"); load(); loadPending(active); }} />
      )}
      {guard && site?.data && (
        <GuardSheet site={site.data} onCancel={() => setGuard(null)}
          onNow={() => { const o = guard; setGuard(null); void startDeploy(active, o); }}
          onBest={() => { const o = guard; setGuard(null); setSchedule({ options: o, preset: "best" }); }}
          onQuiet={() => { const o = guard; setGuard(null); setSchedule({ options: o, preset: "quiet" }); }}
          onSoon={() => { const o = guard; setGuard(null); setSchedule({ options: o, preset: "soon" }); }} />
      )}
      {announcing && <AnnounceSheet profile={active} onClose={() => setAnnouncing(false)} onDone={(t) => { setAnnouncing(false); flash(t); load(); }} />}
      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}
