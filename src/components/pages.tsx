import { useEffect, useState } from "react";
import { api, duration, short, when, type Config, type Release } from "../api";
import { STATUS_LABEL } from "./deploy";
import { IconFolder, IconRefresh, IconUndo } from "./icons";

/* ── History ────────────────────────────────────────────────────────────── */

export function HistoryPage({ profile, current, busy, onRollback }: { profile: string; current: string; busy: boolean; onRollback: (r: Release) => void }) {
  const [list, setList] = useState<Release[] | null>(null);
  const [err, setErr] = useState("");
  const [onlyBad, setOnlyBad] = useState(false);
  const [reload, setReload] = useState(0);
  useEffect(() => {
    let alive = true;
    api.releases(profile).then((l) => { if (alive) { setList(l); setErr(""); } }).catch((e) => { if (alive) setErr(String(e)); });
    return () => { alive = false; };
  }, [profile, reload]);
  const shown = (list ?? []).filter((r) => !onlyBad || !["success", "dry_run"].includes(r.status));
  return (
    <div className="stack">
      <div className="row toolbar">
        <div className="pills">
          <button className={!onlyBad ? "on" : ""} onClick={() => setOnlyBad(false)}>Все</button>
          <button className={onlyBad ? "on" : ""} onClick={() => setOnlyBad(true)}>С ошибками</button>
        </div>
        <span className="grow" />
        <button className="icon-btn" onClick={() => { setList(null); setReload((n) => n + 1); }} title="Обновить"><IconRefresh size={14} /></button>
      </div>
      {err && <p className="err">{err}</p>}
      {!list && !err && <p className="muted small">Загружаю с сервера…</p>}
      {list && !shown.length && <p className="muted small">Пусто.</p>}
      <ul className="releases card">
        {shown.map((r, i) => (
          <li key={i} className={r.sha === current ? "current" : ""}>
            <span className={`rel-dot s-${r.status}`} title={STATUS_LABEL[r.status] ?? r.status} />
            <div className="grow">
              <div className="ellipsis">{r.message || "—"}</div>
              <div className="muted tiny"><code>{short(r.sha)}</code> · {when(r.finished)} · {duration((Date.parse(r.finished) - Date.parse(r.started)) / 1000)}{r.status !== "success" ? ` · ${STATUS_LABEL[r.status] ?? r.status}` : ""}</div>
            </div>
            {r.sha === current ? <span className="tag t-ok">сейчас</span>
              : r.status === "success" && <button className="icon-btn" title="Откатить сюда" disabled={busy} onClick={() => onRollback(r)}><IconUndo size={13} /></button>}
          </li>
        ))}
      </ul>
    </div>
  );
}

/* ── Settings ───────────────────────────────────────────────────────────── */

const DEFAULT_PROBE = "docker compose exec -T backend python3 -m app.deploy_window";

export function SettingsPage({ config, onSaved, onTestSite }: { config: Config; onSaved: () => void; onTestSite: (id: string) => Promise<string> }) {
  const [draft, setDraft] = useState<Config>(config);
  const [saved, setSaved] = useState("");
  const [test, setTest] = useState<Record<string, string>>({});
  const setProfile = (i: number, patch: Partial<Config["profiles"][number]>) =>
    setDraft({ ...draft, profiles: draft.profiles.map((p, j) => (i === j ? { ...p, ...patch } : p)) });
  return (
    <div className="stack">
      <section className="card form">
        <label className="switch"><input type="checkbox" checked={draft.guard_manual} onChange={(e) => setDraft({ ...draft, guard_manual: e.target.checked })} /><span />Спрашивать, если на сайте работают люди</label>
        <label className="field">Баннер перед плановым деплоем
          <select value={draft.announce_minutes} onChange={(e) => setDraft({ ...draft, announce_minutes: Number(e.target.value) })}>
            <option value={0}>не показывать</option>
            {[2, 5, 10, 15].map((m) => <option key={m} value={m}>за {m} мин</option>)}
          </select>
        </label>
        <details>
          <summary>Дополнительно</summary>
          <label className="field">SSH-ключ<input value={draft.ssh_key} onChange={(e) => setDraft({ ...draft, ssh_key: e.target.value })} /></label>
          <div className="two">
            <label className="field">GitHub, сек<input type="number" min={15} value={draft.poll_seconds} onChange={(e) => setDraft({ ...draft, poll_seconds: Number(e.target.value) })} /></label>
            <label className="field">Сайт, сек<input type="number" min={30} value={draft.site_poll_seconds} onChange={(e) => setDraft({ ...draft, site_poll_seconds: Number(e.target.value) })} /></label>
          </div>
        </details>
      </section>

      {draft.profiles.map((p, i) => (
        <details key={p.id} className="card form">
          <summary><b>{p.name}</b><span className="muted tiny">{p.repo_path}</span></summary>
          <label className="field">Папка проекта<input value={p.repo_path} onChange={(e) => setProfile(i, { repo_path: e.target.value })} /></label>
          <label className="field">Папка на сервере<input value={p.remote_dir} onChange={(e) => setProfile(i, { remote_dir: e.target.value })} /></label>
          <label className="field">Проверки перед коммитом
            <textarea rows={2} value={p.checks.join("\n")} placeholder="по одной команде в строке"
              onChange={(e) => setProfile(i, { checks: e.target.value.split("\n").filter((l) => l.trim()) })} />
          </label>
          <label className="switch"><input type="checkbox" checked={p.auto_pull} onChange={(e) => setProfile(i, { auto_pull: e.target.checked })} /><span />Подтягивать изменения с GitHub</label>
          <label className="switch"><input type="checkbox" checked={!!p.site_probe} onChange={(e) => setProfile(i, { site_probe: e.target.checked ? (config.profiles[i].site_probe || DEFAULT_PROBE) : "" })} /><span />Связь с сайтом</label>
          {!!p.site_probe && (
            <div className="row gap">
              <button className="btn sm" onClick={async () => { setTest((t) => ({ ...t, [p.id]: "Проверяю…" })); const r = await onTestSite(p.id); setTest((t) => ({ ...t, [p.id]: r })); }}>Проверить связь</button>
              {test[p.id] && <span className="muted tiny">{test[p.id]}</span>}
            </div>
          )}
          <button className="link" onClick={() => api.openPath(p.repo_path)}><IconFolder size={12} />Открыть папку</button>
        </details>
      ))}

      <div className="row gap">
        <button className="btn primary" onClick={async () => { await api.saveConfig(draft); setSaved("Сохранено"); onSaved(); window.setTimeout(() => setSaved(""), 2500); }}>Сохранить</button>
        {saved && <span className="ok small">{saved}</span>}
      </div>
    </div>
  );
}
