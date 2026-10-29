import { hhmm, plural, until, type Hour, type SiteState, type SiteWindow, type Verdict } from "../api";
import { IconAlert, IconCheck, IconSparkle, IconStop } from "./icons";

export const VERDICT_ICON: Record<Verdict, typeof IconCheck> = { ok: IconCheck, warn: IconAlert, stop: IconStop };
const SLOT_LABEL = { quiet: "тихо", moderate: "средне", busy: "нагружено" };

export function usersText(v: number) {
  if (v < 0.5) return "обычно никого";
  const n = Math.max(1, Math.round(v));
  return `обычно ~${n} ${plural(n, "человек", "человека", "человек")}`;
}

function uptime(seconds: number) {
  if (seconds < 3600) return `${Math.max(1, Math.floor(seconds / 60))} мин`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} ч`;
  return `${Math.floor(seconds / 86400)} д`;
}

export function Timeline({ hours, best }: { hours: Hour[]; best?: SiteWindow }) {
  const max = Math.max(...hours.map((h) => h.load), 0.05);
  return (
    <div className="timeline">
      <div className="tl-bars">
        {hours.map((h, i) => {
          const inBest = best && h.at >= best.start && h.at < best.end;
          return (
            <div key={h.at} className={`tl-bar lv-${h.level}${i === 0 ? " now" : ""}${inBest ? " best" : ""}`}
              title={`${hhmm(h.at)} — ${SLOT_LABEL[h.level]}, ${usersText(h.users)}`}>
              <span style={{ height: `${Math.max(12, (h.load / max) * 100)}%` }} />
            </div>
          );
        })}
      </div>
      <div className="tl-axis">
        {hours.map((h, i) => <span key={h.at}>{i === 0 ? "сейчас" : h.hour % 6 === 0 ? String(h.hour).padStart(2, "0") : ""}</span>)}
      </div>
    </div>
  );
}

/** One glance: can I deploy, who is on, when is ideal. */
export function StatusHero({ state, onIdeal }: { state?: SiteState; onIdeal: () => void }) {
  const data = state?.data;
  if (!data) {
    return (
      <section className="hero v-none">
        <div className="hero-top"><span className="hero-dot" /><div><b>Сайт</b><small>{state?.error || "спрашиваю…"}</small></div></div>
      </section>
    );
  }
  const v = data.verdict;
  const Icon = VERDICT_ICON[v.level];
  const online = data.people.filter((p) => !p.excluded && p.online).length;
  const best = data.windows[0];
  const reasons = v.reasons.filter((r) => r.level === "stop" || r.level === "warn").slice(0, 2);
  return (
    <section className={`hero v-${v.level}`}>
      <div className="hero-top">
        <span className="hero-icon"><Icon size={18} /></span>
        <div className="grow">
          <b>{v.title}</b>
          <small>{online ? `${online} ${plural(online, "человек", "человека", "человек")} онлайн` : "никого онлайн"} · {data.signals.messages_10m} сообщ. за 10 мин · без перезапуска {uptime(data.server.uptime_seconds)}</small>
        </div>
      </div>
      {reasons.length > 0 && <ul className="hero-why">{reasons.map((r) => <li key={r.key}>{r.title}</li>)}</ul>}
      {v.level !== "ok" && best && (
        <button className="hero-ideal" onClick={onIdeal}>
          <IconSparkle size={14} /><span>Идеально: <b>{hhmm(best.start)}</b> · {until(best.start)}</span><span className="go">Запланировать</span>
        </button>
      )}
    </section>
  );
}
