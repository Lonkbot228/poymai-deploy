import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/* Our own minimize / maximize / close, like Discord: the native frame is off
   (tauri.conf.json "decorations": false) and the header is the drag region.
   Close only hides the window — the app keeps living in the tray. */

export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const win = getCurrentWindow();
    const sync = () => { win.isMaximized().then(setMaximized).catch(() => {}); };
    sync();
    const off = win.onResized(sync);
    return () => { off.then((un) => un()).catch(() => {}); };
  }, []);

  const win = () => getCurrentWindow();
  return (
    <div className="win-controls">
      <button className="win-btn" aria-label="Свернуть" title="Свернуть" onClick={() => void win().minimize()}>
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M1 5.5h8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" /></svg>
      </button>
      <button className="win-btn" aria-label={maximized ? "Восстановить" : "Развернуть"} title={maximized ? "Восстановить" : "Развернуть"} onClick={() => void win().toggleMaximize()}>
        {maximized ? (
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.1">
            <rect x="1" y="3" width="6" height="6" rx="1.4" /><path d="M3 3V2.4C3 1.6 3.6 1 4.4 1h3.2C8.4 1 9 1.6 9 2.4v3.2C9 6.4 8.4 7 7.6 7H7" />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.1"><rect x="1" y="1" width="8" height="8" rx="1.8" /></svg>
        )}
      </button>
      <button className="win-btn close" aria-label="Закрыть" title="Закрыть (останется в трее)" onClick={() => void win().close()}>
        <svg width="10" height="10" viewBox="0 0 10 10"><path d="M1.5 1.5l7 7M8.5 1.5l-7 7" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" /></svg>
      </button>
    </div>
  );
}
