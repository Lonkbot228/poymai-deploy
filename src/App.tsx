import { useCallback, useEffect, useRef, useState } from "react";
import {
  api, ago, short,
  type Config, type DeployEvent, type DeployOptions, type Outcome, type ProfileStatus,
  type Release, type ServerStatus, type Snapshot,
} from "./api";

type Tab = "deploy" | "history" | "settings";
type Run = { profile: string; progress: number; step: string; log: DeployEvent[]; outcome?: Outcome; error?: string };

export default function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [active, setActive] = useState("main");
  const [tab, setTab] = useState<Tab>("deploy");
  const [run, setRun] = useState<Run | null>(null);
  const [server, setServer] = useState<Record<string, ServerStatus | string>>({});

  const load = useCallback(() => api.snapshot().then(setSnap), []);
  const loadServer = useCallback((id: string) => {
    api.serverStatus(id)
      .then((s) => setServer((m) => ({ ...m, [id]: s })))
      .catch((e) => setServer((m) => ({ ...m, [id]: String(e) })));
  }, []);

  const startDeploy = useCallback(async (profile: string, options: DeployOptions) => {
    setActive(profile);
    setTab("deploy");
    setRun({ profile, progress: 0, step: "", log: [] });
    try {
      const outcome = await api.deploy(profile, options);
      setRun((r) => r && { ...r, outcome, progress: outcome.ok ? 100 : r.progress });
    } catch (e) {
      setRun((r) => r && { ...r, error: String(e) });
    }
    load();
    loadServer(profile);
  }, [load, loadServer]);

  useEffect(() => {
    load();
    const subs = [
      api.onStatus(load),
      api.onDeployEvent((e) =>
        setRun((r) => {
          if (!r || r.profile !== e.profile) return r;
          return {
            ...r,
            progress: Math.max(r.progress, e.progress),
            step: e.kind === "step" ? e.text : r.step,
            log: [...r.log, e].slice(-800),
          };
        }),
      ),
      api.onTrayDeploy((p) => startDeploy(p, {})),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, [load, startDeploy]);

  useEffect(() => {
    if (snap) loadServer(active);
  }, [active, !!snap]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!snap) return <div className="boot">Загрузка…</div>;
  const status = snap.statuses.find((s) => s.id === active);
  const profile = snap.config.profiles.find((p) => p.id === active)!;
  const busy = snap.busy !== null || (run !== null && !run.outcome && !run.error);

  return (
    <div className="app">
      <header>
        <div className="brand">
          <span className="logo" />
          <div>
            <h1>Poymai Deploy</h1>
            <small>{snap.machine}</small>
          </div>
        </div>
        <button className="ghost" title="Обновить" onClick={() => { api.refresh(); loadServer(active); }}>⟳</button>
      </header>

      <nav className="profiles">
        {snap.config.profiles.map((p) => {
          const st = snap.statuses.find((s) => s.id === p.id);
          return (
            <button key={p.id} className={p.id === active ? "on" : ""} onClick={() => setActive(p.id)}>
              <Dot tone={tone(st)} /> {p.name}
            </button>
          );
        })}
      </nav>

      <nav className="tabs">
        {([["deploy", "Деплой"], ["history", "История"], ["settings", "Настройки"]] as [Tab, string][]).map(([id, label]) => (
          <button key={id} className={tab === id ? "on" : ""} onClick={() => setTab(id)}>{label}</button>
        ))}
      </nav>

      <main>
        {tab === "deploy" && (
          <>
            <SyncCard status={status} server={server[active]} />
            {status && status.incoming > 0 && <IncomingCard status={status} onDone={load} />}
            {run && run.profile === active ? (
              <RunCard run={run} steps={snap.steps} onClose={() => { setRun(null); api.dismissError(); }}
                onRetry={(o) => startDeploy(active, o)} />
            ) : (
              <DeployCard status={status} busy={busy} onDeploy={(o) => startDeploy(active, o)} />
            )}
          </>
        )}
        {tab === "history" && (
          <History profile={active} current={typeof server[active] === "object" ? (server[active] as ServerStatus).head : ""}
            busy={busy} onRollback={(sha) => startDeploy(active, { sha })} />
        )}
        {tab === "settings" && <Settings config={snap.config} onSaved={load} repo={profile.repo_path} />}
      </main>
    </div>
  );
}

function tone(st?: ProfileStatus): "ok" | "warn" | "bad" | "idle" {
  if (!st) return "idle";
  if (st.error) return "bad";
  if (st.incoming > 0) return "warn";
  if (st.local && (st.local.changes.length > 0 || st.local.head !== st.production)) return "warn";
  return "ok";
}

function Dot({ tone }: { tone: string }) {
  return <span className={`dot ${tone}`} />;
}

function SyncCard({ status, server }: { status?: ProfileStatus; server?: ServerStatus | string }) {
  if (!status) return <section className="card">Проверяю…</section>;
  const local = status.local;
  const srv = typeof server === "object" ? server : null;
  const serverSha = srv?.head || status.production;
  const nodes = [
    { label: "Этот компьютер", sha: local?.head, sub: local ? (local.changes.length ? `${local.changes.length} изм. файлов` : "чисто") : "—", ok: !!local && local.changes.length === 0 && local.head === status.github },
    { label: "GitHub", sha: status.github, sub: "main", ok: !!status.github && status.github === serverSha },
    { label: "Сервер", sha: serverSha, sub: srv ? srv.route : typeof server === "string" ? "нет связи" : "…", ok: !!serverSha && serverSha === status.github && (srv?.drift ?? 0) === 0 },
  ];
  let summary = "Всё синхронизировано";
  let summaryTone = "ok";
  if (status.error) { summary = status.error; summaryTone = "bad"; }
  else if (status.incoming > 0) { summary = `На GitHub ${status.incoming} новых коммитов с другого компьютера`; summaryTone = "warn"; }
  else if (local && local.changes.length > 0) { summary = "Есть незакоммиченные изменения"; summaryTone = "warn"; }
  else if (local && local.ahead > 0) { summary = `${local.ahead} коммитов не отправлено в GitHub`; summaryTone = "warn"; }
  else if (status.github && serverSha && status.github !== serverSha) { summary = "На сервере не последняя версия из GitHub"; summaryTone = "warn"; }
  if (srv && srv.drift > 0) { summary = `На сервере ${srv.drift} файлов изменено вручную`; summaryTone = "bad"; }
  if (srv?.busy) { summary = `Идёт деплой: ${srv.busy}`; summaryTone = "warn"; }

  return (
    <section className="card">
      <div className="chain">
        {nodes.map((n, i) => (
          <div key={n.label} className="chain-item">
            <div className={`node ${n.ok ? "ok" : "warn"}`}>
              <span className="node-label">{n.label}</span>
              <code>{short(n.sha)}</code>
              <span className="node-sub">{n.sub}</span>
            </div>
            {i < nodes.length - 1 && <span className="link" />}
          </div>
        ))}
      </div>
      <p className={`summary ${summaryTone}`}>{summary}</p>
      {srv?.current && (
        <p className="muted">
          На сервере: «{srv.current.message}» · {srv.current.actor} · {srv.current.machine} · {ago(srv.current.finished)}
        </p>
      )}
      <p className="muted tiny">Проверено в {status.checked_at}</p>
    </section>
  );
}

function IncomingCard({ status, onDone }: { status: ProfileStatus; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  return (
    <section className="card accent">
      <h3>Обновления с другого компьютера</h3>
      {status.overlap.length > 0 ? (
        <p className="bad">Эти файлы изменены и у вас, и в обновлении — разрулите вручную: {status.overlap.join(", ")}</p>
      ) : (
        <p className="muted">Ваши локальные правки будут сохранены и возвращены после обновления.</p>
      )}
      <button disabled={busy || status.overlap.length > 0} onClick={async () => {
        setBusy(true);
        try { setMsg(await api.pullUpdates(status.id)); } catch (e) { setMsg(String(e)); }
        setBusy(false);
        onDone();
      }}>{busy ? "Обновляю…" : `Получить ${status.incoming} коммит(ов)`}</button>
      {msg && <p className="muted pre">{msg}</p>}
    </section>
  );
}

function DeployCard({ status, busy, onDeploy }: { status?: ProfileStatus; busy: boolean; onDeploy: (o: DeployOptions) => void }) {
  const [message, setMessage] = useState("");
  const [dry, setDry] = useState(false);
  const [backup, setBackup] = useState(false);
  const [showFiles, setShowFiles] = useState(false);
  const changes = status?.local?.changes ?? [];
  const blocked = !status?.local || status.incoming > 0 && status.overlap.length > 0;
  return (
    <section className="card">
      <div className="row between">
        <h3>Новый деплой</h3>
        {changes.length > 0 && (
          <button className="link-btn" onClick={() => setShowFiles(!showFiles)}>
            {changes.length} файлов {showFiles ? "▴" : "▾"}
          </button>
        )}
      </div>
      {showFiles && (
        <ul className="files">
          {changes.map((f) => (
            <li key={f.path}><span className={`fs fs-${f.status.replace("?", "A")[0]}`}>{f.status === "??" ? "A" : f.status}</span>{f.path}</li>
          ))}
        </ul>
      )}
      <textarea
        placeholder={changes.length ? "Что изменилось? (пусто — сообщение сгенерируется)" : "Нет новых изменений — будет передеплоен текущий коммит"}
        value={message}
        onChange={(e) => setMessage(e.target.value)}
        rows={2}
      />
      <div className="row opts">
        <label><input type="checkbox" checked={dry} onChange={(e) => setDry(e.target.checked)} /> Только проверить</label>
        <label><input type="checkbox" checked={backup} onChange={(e) => setBackup(e.target.checked)} /> Бэкап БД</label>
      </div>
      <button className="primary" disabled={busy || blocked}
        onClick={() => onDeploy({ message, dry_run: dry, backup_db: backup })}>
        {busy ? "Деплой идёт…" : dry ? "Проверить" : "Задеплоить"}
      </button>
      <p className="muted tiny">Коммит → GitHub → сервер ставит этот коммит → проверка здоровья → при ошибке автоматический откат.</p>
    </section>
  );
}

function RunCard({ run, steps, onClose, onRetry }: {
  run: Run; steps: [string, number, string][]; onClose: () => void; onRetry: (o: DeployOptions) => void;
}) {
  const logRef = useRef<HTMLDivElement>(null);
  useEffect(() => { logRef.current?.scrollTo(0, logRef.current.scrollHeight); }, [run.log.length]);
  const done = run.outcome?.ok;
  const failed = run.error || (run.outcome && !run.outcome.ok);
  const errText = run.error || (!run.outcome?.ok ? run.outcome?.text : "");
  const seen = new Set(run.log.filter((e) => e.kind === "step").map((e) => e.step));
  const current = [...run.log].reverse().find((e) => e.kind === "step")?.step;
  const visible = steps.filter(([id]) => seen.has(id));
  const logText = run.log.map((e) => (e.kind === "step" ? `==> ${e.text}` : e.text)).join("\n");

  return (
    <section className={`card run ${done ? "ok" : failed ? "bad" : ""}`}>
      <div className="row between">
        <h3>{done ? "Готово" : failed ? "Ошибка" : run.step || "Запуск…"}</h3>
        <span className="pct">{run.progress}%</span>
      </div>
      <div className="bar"><span style={{ width: `${run.progress}%` }} /></div>
      <ol className="steps">
        {visible.map(([id, , label]) => (
          <li key={id} className={id === current && !done ? (failed ? "bad" : "now") : "past"}>{label}</li>
        ))}
      </ol>
      {errText && <p className="bad pre">{errText}</p>}
      {done && <p className="ok">{run.outcome!.text}</p>}
      <div className="log" ref={logRef}>
        {run.log.filter((e) => e.kind !== "step").map((e, i) => (
          <div key={i} className={`l-${e.kind}`}>{e.text}</div>
        ))}
      </div>
      <div className="row gap">
        {(done || failed) && <button onClick={onClose}>Закрыть</button>}
        <button className="ghost" onClick={() => navigator.clipboard.writeText(logText + (errText ? `\n\n${errText}` : ""))}>Копировать лог</button>
        {run.outcome?.code === 20 && (
          <button className="danger" onClick={() => onRetry({ accept_drift: true, sha: run.outcome!.sha })}>
            Перезаписать правки на сервере
          </button>
        )}
      </div>
    </section>
  );
}

function History({ profile, current, busy, onRollback }: { profile: string; current: string; busy: boolean; onRollback: (sha: string) => void }) {
  const [list, setList] = useState<Release[] | null>(null);
  const [err, setErr] = useState("");
  useEffect(() => {
    setList(null);
    api.releases(profile).then(setList).catch((e) => setErr(String(e)));
  }, [profile]);
  if (err) return <section className="card bad">{err}</section>;
  if (!list) return <section className="card">Загружаю историю с сервера…</section>;
  if (!list.length) return <section className="card muted">Релизов пока нет — история появится после первого деплоя через приложение.</section>;
  const label: Record<string, string> = {
    success: "успех", rolled_back: "откат", build_failed: "сборка упала", failed: "ошибка",
    rollback_failed: "откат не удался", dry_run: "проверка",
  };
  return (
    <section className="card">
      <ul className="releases">
        {list.map((r, i) => (
          <li key={i} className={r.sha === current ? "current" : ""}>
            <div className="row between">
              <span><span className={`badge s-${r.status}`}>{label[r.status] ?? r.status}</span> <code>{short(r.sha)}</code></span>
              <span className="muted tiny">{ago(r.finished)}</span>
            </div>
            <div className="msg">{r.message}</div>
            <div className="row between">
              <span className="muted tiny">{r.actor} · {r.machine}</span>
              {r.status === "success" && r.sha !== current && (
                <button className="link-btn" disabled={busy}
                  onClick={() => confirm(`Откатить сервер на ${short(r.sha)} «${r.message}»?`) && onRollback(r.sha)}>
                  Откатить сюда
                </button>
              )}
              {r.sha === current && <span className="tiny ok">сейчас на сервере</span>}
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

function Settings({ config, onSaved, repo }: { config: Config; onSaved: () => void; repo: string }) {
  const [draft, setDraft] = useState<Config>(config);
  const [saved, setSaved] = useState("");
  const setProfile = (i: number, patch: Partial<Config["profiles"][number]>) =>
    setDraft({ ...draft, profiles: draft.profiles.map((p, j) => (i === j ? { ...p, ...patch } : p)) });
  return (
    <section className="card settings">
      {draft.profiles.map((p, i) => (
        <fieldset key={p.id}>
          <legend>{p.name}</legend>
          <label>Папка проекта на этом компьютере<input value={p.repo_path} onChange={(e) => setProfile(i, { repo_path: e.target.value })} /></label>
          <label>Папка на сервере<input value={p.remote_dir} onChange={(e) => setProfile(i, { remote_dir: e.target.value })} /></label>
          <label>Проверки перед коммитом (по строке)
            <textarea rows={2} value={p.checks.join("\n")} placeholder="например: npm --prefix frontend run lint"
              onChange={(e) => setProfile(i, { checks: e.target.value.split("\n").filter((l) => l.trim()) })} />
          </label>
          <label className="check"><input type="checkbox" checked={p.auto_pull} onChange={(e) => setProfile(i, { auto_pull: e.target.checked })} /> Автоматически подтягивать изменения с GitHub</label>
          <p className="muted tiny">Сервер: {p.routes.map((r) => `${r.name}: ${r.user}@${r.host}${r.jump ? ` через ${r.jump}` : ""}`).join(" → ")}</p>
        </fieldset>
      ))}
      <label>SSH-ключ<input value={draft.ssh_key} onChange={(e) => setDraft({ ...draft, ssh_key: e.target.value })} /></label>
      <label>Проверять GitHub каждые, сек<input type="number" min={15} value={draft.poll_seconds}
        onChange={(e) => setDraft({ ...draft, poll_seconds: Number(e.target.value) })} /></label>
      <div className="row gap">
        <button className="primary" onClick={async () => { await api.saveConfig(draft); setSaved("Сохранено"); onSaved(); }}>Сохранить</button>
        <button className="ghost" onClick={() => api.openPath(repo)}>Открыть папку проекта</button>
      </div>
      {saved && <p className="ok">{saved}</p>}
      <p className="muted tiny">Настройки этого компьютера: ~/.poymai-deploy/config.json</p>
    </section>
  );
}
