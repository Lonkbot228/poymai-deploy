import { useEffect, useMemo, useState } from "react";
import {
  api, ago, fromLocalInput, hhmm, short, toLocalInput, until, when,
  type DeployEvent, type DeployOptions, type Job, type Plan, type SiteData, type SiteState,
} from "../api";
import { IconBell, IconCalendar, IconCheck, IconChevron, IconClock, IconMoon, IconPlay, IconSparkle, IconX } from "./icons";
import { Sheet } from "./modals";
import { Timeline, usersText } from "./site";

/* ── Presets: the ideal time first, then the usual choices ─────────────── */

export type PresetId = "best" | "best2" | "best3" | "quiet" | "soon" | "night" | "custom";
export type Preset = { id: PresetId; icon: typeof IconClock; title: string; sub: string; ideal?: boolean; plan: () => Plan };

function nightAt3() {
  const d = new Date();
  d.setHours(3, 0, 0, 0);
  if (d.getTime() < Date.now() + 15 * 60_000) d.setDate(d.getDate() + 1);
  return d;
}

export function presets(site: SiteData | null | undefined, announce: number): Preset[] {
  const list: Preset[] = [];
  const banner = site ? announce : 0;
  (site?.windows ?? []).slice(0, 3).forEach((w, i) => list.push({
    id: (["best", "best2", "best3"] as const)[i],
    icon: IconSparkle,
    ideal: i === 0,
    title: `${when(w.start)}`,
    sub: `${i === 0 ? "идеальное время · " : ""}${usersText(w.users)}`,
    plan: () => ({ mode: "at", run_at: w.start, announce_minutes: banner }),
  }));
  if (site) list.push({
    id: "quiet", icon: IconMoon, title: "Когда никого нет", sub: "сам дождётся тишины, максимум 12 часов",
    plan: () => ({ mode: "quiet", deadline: new Date(Date.now() + 12 * 3600_000).toISOString(), announce_minutes: banner }),
  });
  list.push({
    id: "soon", icon: IconBell, title: "Через 5 минут", sub: site ? "сразу покажет баннер пользователям" : "небольшая задержка",
    plan: () => ({ mode: "at", run_at: new Date(Date.now() + 5 * 60_000).toISOString(), announce_minutes: site ? 5 : 0 }),
  });
  if (!site?.windows.length) list.push({
    id: "night", icon: IconMoon, title: when(nightAt3().toISOString()), sub: "ночью, когда обычно тихо",
    plan: () => ({ mode: "at", run_at: nightAt3().toISOString(), announce_minutes: banner }),
  });
  return list;
}

/* ── Sheet: pick a preset, confirm ─────────────────────────────────────── */

export function ScheduleSheet({ profile, name, site, initial, preset, announce, onClose, onDone }: {
  profile: string;
  name: string;
  site?: SiteData | null;
  initial: DeployOptions;
  preset?: PresetId;
  announce: number;
  onClose: () => void;
  onDone: (text: string) => void;
}) {
  const list = useMemo(() => presets(site, announce), [site, announce]);
  const [choice, setChoice] = useState<PresetId>(preset && (preset === "custom" || list.some((p) => p.id === preset)) ? preset : list[0].id);
  const [custom, setCustom] = useState(() => toLocalInput(new Date(Date.now() + 3600_000)));
  const [message, setMessage] = useState(initial.message ?? "");
  const [backup, setBackup] = useState(!!initial.backup_db);
  const [busy, setBusy] = useState(false);
  const [step, setStep] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    const sub = api.onPublishEvent((e: DeployEvent) => { if (e.profile === profile) setStep(e.text); });
    return () => { sub.then((un) => un()); };
  }, [profile]);

  const submit = async () => {
    const base: Plan = choice === "custom"
      ? { mode: "at", run_at: fromLocalInput(custom), announce_minutes: site ? announce : 0 }
      : list.find((p) => p.id === choice)!.plan();
    if (base.run_at && Date.parse(base.run_at) < Date.now() - 60_000) { setError("Это время уже прошло"); return; }
    setBusy(true);
    setError("");
    try {
      const job = await api.schedule(profile, { ...base, options: { ...initial, message, backup_db: backup, dry_run: false } });
      onDone(`${name}: ${short(job.sha)} запланирован`);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <Sheet title="Когда задеплоить?" onClose={busy ? () => {} : onClose}>
      <div className="options">
        {list.map((p) => (
          <button key={p.id} className={`option${choice === p.id ? " on" : ""}${p.ideal ? " ideal" : ""}`} onClick={() => setChoice(p.id)}>
            <p.icon size={16} />
            <span><b>{p.title}</b><small>{p.sub}</small></span>
            {choice === p.id && <IconCheck size={15} className="option-check" />}
          </button>
        ))}
        <button className={`option${choice === "custom" ? " on" : ""}`} onClick={() => setChoice("custom")}>
          <IconCalendar size={16} />
          <span><b>Другое время</b><small>{choice === "custom" ? until(fromLocalInput(custom)) : "выбрать дату и час"}</small></span>
        </button>
        {choice === "custom" && <input type="datetime-local" value={custom} onChange={(e) => setCustom(e.target.value)} />}
      </div>
      <input className="msg" value={message} onChange={(e) => setMessage(e.target.value)} placeholder="Сообщение коммита (необязательно)" />
      <label className="switch"><input type="checkbox" checked={backup} onChange={(e) => setBackup(e.target.checked)} /><span />Бэкап БД</label>
      {error && <p className="err">{error}</p>}
      <button className="btn primary block" disabled={busy} onClick={submit}>
        <IconCalendar size={15} />{busy ? step || "Фиксирую коммит…" : "Запланировать"}
      </button>
      <p className="note">Изменения сразу уйдут в GitHub — позже на сервер встанет ровно этот коммит.</p>
    </Sheet>
  );
}

/* ── Plan page ──────────────────────────────────────────────────────────── */

const JOB: Record<string, { label: string; tone: string }> = {
  waiting: { label: "ждёт", tone: "info" },
  announced: { label: "баннер показан", tone: "warn" },
  running: { label: "идёт", tone: "warn" },
  done: { label: "выполнен", tone: "ok" },
  failed: { label: "ошибка", tone: "bad" },
  cancelled: { label: "отменён", tone: "muted" },
  expired: { label: "не выполнен", tone: "bad" },
};

function jobWhen(j: Job, now: number) {
  if (j.announced_for) return `${when(j.announced_for)} · ${until(j.announced_for, now)}`;
  if (j.mode === "at") return `${when(j.run_at)} · ${until(j.run_at, now)}`;
  return `когда никого нет${j.deadline ? ` · до ${when(j.deadline)}` : ""}`;
}

export function PlanPage({ site, hasSite, jobs, names, busy, announce, onPick, onBanner, onClearBanner }: {
  site?: SiteState;
  hasSite: boolean;
  jobs: Job[];
  names: Record<string, string>;
  busy: boolean;
  announce: number;
  onPick: (id: PresetId) => void;
  onBanner: () => void;
  onClearBanner: () => void;
}) {
  const [now, setNow] = useState(() => Date.now());
  const [showDone, setShowDone] = useState(false);
  useEffect(() => { const t = window.setInterval(() => setNow(Date.now()), 15_000); return () => window.clearInterval(t); }, []);
  const data = site?.data;
  const best = data?.windows[0];
  const list = presets(data, announce);
  const active = jobs.filter((j) => ["waiting", "announced", "running"].includes(j.status));
  const done = jobs.filter((j) => !active.includes(j)).sort((a, b) => (b.finished_at ?? "").localeCompare(a.finished_at ?? ""));

  return (
    <div className="stack">
      {hasSite && (
        <section className="card ideal-card">
          <div className="label"><IconSparkle size={13} /> Идеальное время</div>
          {best ? (
            <>
              <button className="ideal-main" onClick={() => onPick("best")}>
                <b>{when(best.start)}–{hhmm(best.end)}</b>
                <small>{until(best.start, now)} · {usersText(best.users)}</small>
                <span className="go"><IconCalendar size={14} />Запланировать</span>
              </button>
              <Timeline hours={data!.timeline} best={best} />
            </>
          ) : <p className="muted">{site?.error || "Спрашиваю сайт…"}</p>}
        </section>
      )}

      <section className="card">
        <div className="label">Быстро запланировать</div>
        <div className="quick">
          {list.filter((p) => p.id !== "best").slice(0, 4).map((p) => (
            <button key={p.id} disabled={busy} onClick={() => onPick(p.id)}><p.icon size={15} /><span><b>{p.title}</b><small>{p.sub}</small></span></button>
          ))}
          <button disabled={busy} onClick={() => onPick("custom")}><IconCalendar size={15} /><span><b>Своё время</b><small>дата и час</small></span></button>
        </div>
      </section>

      <section className="card">
        <div className="label">Запланировано{active.length ? ` · ${active.length}` : ""}</div>
        {!active.length && <p className="muted small">Ничего не ждёт. Выберите вариант выше — приложение само задеплоит и предупредит пользователей.</p>}
        <ul className="jobs">
          {active.map((j) => { const st = JOB[j.status]; return (
            <li key={j.id}>
              {j.mode === "quiet" ? <IconMoon size={16} /> : <IconClock size={16} />}
              <div className="grow">
                <div className="row"><b>{names[j.profile] ?? j.profile}</b><span className={`tag t-${st.tone}`}>{st.label}</span></div>
                <div className="small">{jobWhen(j, now)}</div>
                <div className="muted tiny ellipsis"><code>{short(j.sha)}</code> {j.message}</div>
              </div>
              <button className="icon-btn" title="Задеплоить сейчас" disabled={busy || j.status === "running"} onClick={() => api.runJobNow(j.id)}><IconPlay size={13} /></button>
              <button className="icon-btn" title="Отменить" disabled={j.status === "running"} onClick={() => api.cancelJob(j.id)}><IconX size={13} /></button>
            </li>
          ); })}
        </ul>
        {done.length > 0 && (
          <>
            <button className="link" onClick={() => setShowDone(!showDone)}>Выполненные · {done.length} <IconChevron size={12} /></button>
            {showDone && (
              <ul className="jobs done">
                {done.slice(0, 12).map((j) => { const st = JOB[j.status] ?? { label: j.status, tone: "muted" }; return (
                  <li key={j.id}>
                    <div className="grow">
                      <div className="row"><b>{names[j.profile] ?? j.profile}</b><span className={`tag t-${st.tone}`}>{st.label}</span><span className="muted tiny">{ago(j.finished_at)}</span></div>
                      <div className="muted tiny ellipsis">{j.result || j.message}</div>
                    </div>
                  </li>
                ); })}
              </ul>
            )}
          </>
        )}
      </section>

      {hasSite && (
        <div className="banner-row">
          <IconBell size={14} />
          {data?.notice
            ? <><span className="grow">Баннер: обновление {when(data.notice.starts_at)}</span><button className="link" onClick={onClearBanner}>Убрать</button></>
            : <><span className="grow muted">Баннер пользователям не показан</span><button className="link" onClick={onBanner}>Показать</button></>}
        </div>
      )}
    </div>
  );
}
