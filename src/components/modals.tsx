import { useEffect, useState, type ReactNode } from "react";
import { api, hhmm, until, type SiteData } from "../api";
import { IconBell, IconMoon, IconRocket, IconSparkle, IconX } from "./icons";

/** Bottom sheet: fits the compact window better than a centred dialog. */
export function Sheet({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="sheet" role="dialog" aria-modal="true" aria-label={title}>
        <div className="sheet-head"><h3>{title}</h3><button className="icon-btn" onClick={onClose} aria-label="Закрыть"><IconX size={14} /></button></div>
        {children}
      </div>
    </div>
  );
}

/** Before a manual deploy while people work on the site. */
export function GuardSheet({ site, onCancel, onNow, onBest, onQuiet, onSoon }: {
  site: SiteData;
  onCancel: () => void;
  onNow: () => void;
  onBest: () => void;
  onQuiet: () => void;
  onSoon: () => void;
}) {
  const v = site.verdict;
  const best = site.windows[0];
  const reasons = v.reasons.filter((r) => r.level !== "info").slice(0, 3);
  return (
    <Sheet title={v.title} onClose={onCancel}>
      <ul className={`why v-${v.level}`}>{reasons.map((r) => <li key={r.key}>{r.title}</li>)}</ul>
      <div className="options">
        {best && (
          <button className="option ideal" onClick={onBest}>
            <IconSparkle size={16} /><span><b>В идеальное время · {hhmm(best.start)}</b><small>{until(best.start)}, обычно тихо</small></span>
          </button>
        )}
        <button className="option" onClick={onQuiet}><IconMoon size={16} /><span><b>Когда никого не будет</b><small>приложение дождётся тишины</small></span></button>
        <button className="option" onClick={onSoon}><IconBell size={16} /><span><b>Предупредить и через 5 минут</b><small>все увидят баннер</small></span></button>
        <button className="option danger" onClick={onNow}><IconRocket size={16} /><span><b>Всё равно сейчас</b><small>панель у всех переподключится</small></span></button>
      </div>
    </Sheet>
  );
}

export function AnnounceSheet({ profile, onClose, onDone }: { profile: string; onClose: () => void; onDone: (t: string) => void }) {
  const [minutes, setMinutes] = useState(5);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return (
    <Sheet title="Баннер пользователям" onClose={onClose}>
      <div className="pills">{[2, 5, 10, 15, 30].map((m) => <button key={m} className={minutes === m ? "on" : ""} onClick={() => setMinutes(m)}>{m} мин</button>)}</div>
      <input className="msg" value={text} maxLength={300} onChange={(e) => setText(e.target.value)} placeholder="Текст (необязательно)" />
      <p className="note">«Обновление через {minutes} мин» увидят все в панели. Исчезнет сам.</p>
      {error && <p className="err">{error}</p>}
      <button className="btn primary block" disabled={busy} onClick={async () => {
        setBusy(true);
        try { await api.announce(profile, minutes, text); onDone("Баннер показан"); }
        catch (e) { setError(String(e)); setBusy(false); }
      }}><IconBell size={15} />Показать</button>
    </Sheet>
  );
}
