import { useEffect, useRef, useState } from "react";
import {
  api, ago, duration, short,
  type DeployEvent, type DeployOptions, type Outcome, type Pending, type ProfileStatus, type ServerStatus,
} from "../api";
import { IconCalendar, IconCheck, IconChevron, IconCopy, IconDatabase, IconRocket, IconStop, IconX } from "./icons";

/* ── Computer → GitHub → Server, one line ──────────────────────────────── */

export function SyncLine({ status, server }: { status?: ProfileStatus; server?: ServerStatus | string }) {
  if (!status) return <section className="card sync"><span className="muted small">Проверяю репозиторий…</span></section>;
  const local = status.local;
  const srv = typeof server === "object" ? server : null;
  const serverSha = srv?.head || status.production;
  const nodes = [
    { label: "ПК", sha: local?.head, ok: !!local && !local.changes.length && local.head === status.github },
    { label: "GitHub", sha: status.github, ok: !!status.github && status.github === serverSha },
    { label: "Сервер", sha: serverSha, ok: !!serverSha && serverSha === status.github && (srv?.drift ?? 0) === 0 },
  ];
  let summary = "Всё синхронизировано", tone = "ok";
  if (status.error) { summary = status.error; tone = "bad"; }
  else if (status.incoming > 0) { summary = `+${status.incoming} коммит. с другого компьютера`; tone = "warn"; }
  else if (local?.changes.length) { summary = `${local.changes.length} изменённых файлов`; tone = "warn"; }
  else if (local?.ahead) { summary = `${local.ahead} не отправлено в GitHub`; tone = "warn"; }
  else if (status.github && serverSha && status.github !== serverSha) { summary = "На сервере не последняя версия"; tone = "warn"; }
  if (srv && srv.drift > 0) { summary = `На сервере ${srv.drift} ручных правок`; tone = "bad"; }
  if (srv?.busy) { summary = `Идёт деплой: ${srv.busy}`; tone = "warn"; }
  return (
    <section className="card sync">
      <div className="chain">
        {nodes.map((n, i) => (
          <span key={n.label} className="node">
            {i > 0 && <span className="arrow">→</span>}
            <span className={`dot ${n.ok ? "ok" : "warn"}`} />
            <span className="node-label">{n.label}</span>
            <code>{short(n.sha)}</code>
          </span>
        ))}
      </div>
      <div className={`sync-sum ${tone}`}>{summary}{srv?.current && tone === "ok" ? <span className="muted"> · {ago(srv.current.finished)}</span> : null}</div>
    </section>
  );
}

export function IncomingCard({ status, onDone }: { status: ProfileStatus; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  return (
    <section className="card inline warn-edge">
      <span className="grow small">{status.overlap.length ? `Конфликт файлов: ${status.overlap.join(", ")}` : `На GitHub ${status.incoming} новых коммитов`}</span>
      <button className="btn sm" disabled={busy || status.overlap.length > 0} onClick={async () => {
        setBusy(true);
        try { setMsg(await api.pullUpdates(status.id)); } catch (e) { setMsg(String(e)); }
        setBusy(false);
        onDone();
      }}>{busy ? "…" : "Получить"}</button>
      {msg && <p className="note full">{msg}</p>}
    </section>
  );
}

/* ── What will ship, folded ─────────────────────────────────────────────── */

export function PendingLine({ pending, error }: { pending?: Pending; error?: string }) {
  const [open, setOpen] = useState(false);
  if (error) return null;
  if (!pending) return null;
  const nothing = !pending.commits.length && !pending.local_files;
  const warn = pending.flags.find((f) => f.level === "warn");
  return (
    <section className="card fold">
      <button className="fold-head" onClick={() => setOpen(!open)} disabled={nothing}>
        <span className="grow">
          {nothing ? <span className="muted">На сервере уже последняя версия</span> : <>
            Уедет: <b>{pending.commits.length}</b> коммит. · <b>{pending.files.length}</b> файлов
            {pending.migrations.length > 0 && <span className="tag t-warn"><IconDatabase size={11} />миграция</span>}
          </>}
        </span>
        {!nothing && <IconChevron size={14} className={open ? "flip" : ""} />}
      </button>
      {open && (
        <div className="fold-body">
          <div className="pills static">{pending.areas.map((a) => <span key={a.name}>{a.name} {a.count}</span>)}</div>
          {warn && <p className="note warn">{warn.text}</p>}
          <ul className="commits">
            {pending.local_files > 0 && <li><b>Незакоммиченные изменения</b><small>{pending.local_files} файлов</small></li>}
            {pending.commits.slice(0, 8).map((c) => <li key={c.sha}><b>{c.subject}</b><small><code>{short(c.sha)}</code> · {ago(c.date)}</small></li>)}
          </ul>
        </div>
      )}
    </section>
  );
}

/* ── Composer ───────────────────────────────────────────────────────────── */

export function Composer({ status, busy, onDeploy, onSchedule }: {
  status?: ProfileStatus;
  busy: boolean;
  onDeploy: (o: DeployOptions) => void;
  onSchedule: (o: DeployOptions) => void;
}) {
  const [message, setMessage] = useState("");
  const [dry, setDry] = useState(false);
  const [backup, setBackup] = useState(false);
  const changes = status?.local?.changes.length ?? 0;
  const suggested = status?.local?.suggested_message ?? "";
  // Local changes are committed only with a message the user has seen.
  const needsMessage = changes > 0 && !message.trim();
  const blocked = busy || !status?.local || (status.incoming > 0 && status.overlap.length > 0) || needsMessage;
  const opts = (): DeployOptions => ({ message, dry_run: dry, backup_db: backup });
  return (
    <section className="card composer">
      <input className="msg" value={message} onChange={(e) => setMessage(e.target.value)}
        placeholder={changes ? "Что изменилось?" : "Изменений нет — передеплой текущего коммита"} />
      {needsMessage && suggested && (
        <p className="muted tiny">
          Предлагаемое сообщение: «{suggested}»{" "}
          <button className="link-btn" onClick={() => setMessage(suggested)}>Подставить</button>
        </p>
      )}
      <div className="composer-row">
        <button className={`toggle${dry ? " on" : ""}`} onClick={() => setDry(!dry)} title="Проверить без перезапуска">Проверка</button>
        <button className={`toggle${backup ? " on" : ""}`} onClick={() => setBackup(!backup)} title="Сделать бэкап базы перед деплоем">Бэкап БД</button>
        <span className="grow" />
        <button className="btn icon" disabled={blocked || dry} onClick={() => onSchedule(opts())} title="Запланировать"><IconCalendar size={16} /></button>
        <button className="btn primary" disabled={blocked} onClick={() => onDeploy(opts())}>
          <IconRocket size={15} />{busy ? "Идёт…" : dry ? "Проверить" : "Задеплоить"}
        </button>
      </div>
    </section>
  );
}

/* ── Live run ───────────────────────────────────────────────────────────── */

export type Run = { profile: string; progress: number; step: string; log: DeployEvent[]; outcome?: Outcome; error?: string; started: number };

export function RunCard({ run, onClose, onRetry }: { run: Run; onClose: () => void; onRetry: (o: DeployOptions) => void }) {
  const logRef = useRef<HTMLDivElement>(null);
  const [now, setNow] = useState(() => Date.now());
  const [showLog, setShowLog] = useState(false);
  const done = run.outcome?.ok;
  const failed = !!run.error || (run.outcome && !run.outcome.ok);
  useEffect(() => { logRef.current?.scrollTo(0, logRef.current.scrollHeight); }, [run.log.length, showLog]);
  useEffect(() => {
    if (done || failed) return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [done, failed]);
  useEffect(() => { if (failed) setShowLog(true); }, [failed]);
  const errText = run.error || (!run.outcome?.ok ? run.outcome?.text : "");
  const logText = run.log.map((e) => (e.kind === "step" ? `==> ${e.text}` : e.text)).join("\n");
  const elapsed = run.outcome ? run.outcome.seconds : Math.round((now - run.started) / 1000);
  return (
    <section className={`card run${done ? " ok" : failed ? " bad" : ""}`}>
      <div className="row">
        {done ? <IconCheck size={16} className="ok" /> : failed ? <IconStop size={16} className="bad" /> : <span className="pulse" />}
        <b className="grow ellipsis">{done ? run.outcome!.text : failed ? "Деплой не удался" : run.step || "Запуск…"}</b>
        <span className="muted tiny">{duration(elapsed)} · {run.progress}%</span>
      </div>
      <div className="bar"><span style={{ width: `${run.progress}%` }} /></div>
      {errText && <p className="err">{errText}</p>}
      {showLog && <div className="log" ref={logRef}>{run.log.filter((e) => e.kind !== "step").map((e, i) => <div key={i} className={`l-${e.kind}`}>{e.text}</div>)}</div>}
      <div className="row gap">
        <button className="link" onClick={() => setShowLog(!showLog)}>{showLog ? "Скрыть лог" : "Лог"}</button>
        <button className="link" onClick={() => navigator.clipboard.writeText(logText + (errText ? `\n\n${errText}` : ""))}><IconCopy size={12} />Копировать</button>
        <span className="grow" />
        {done && <span className="muted tiny">закроется само через 5 мин</span>}
        {run.outcome?.code === 20 && <button className="btn sm danger" onClick={() => onRetry({ accept_drift: true, sha: run.outcome!.sha })}>Перезаписать правки</button>}
        {(done || failed) && <button className="btn sm" onClick={onClose}><IconX size={12} />Закрыть</button>}
      </div>
    </section>
  );
}

export const STATUS_LABEL: Record<string, string> = {
  success: "успех", rolled_back: "откат", build_failed: "сборка упала", failed: "ошибка",
  rollback_failed: "откат не удался", dry_run: "проверка",
};
