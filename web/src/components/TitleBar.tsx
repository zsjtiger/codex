import { windowControls } from "../desktop";
import { tx } from "../i18n";

/** The desktop window's own title bar. Dragging the empty area moves the window. */
export function TitleBar() {
  return (
    <div className="desktop-titlebar" data-tauri-drag-region>
      <span className="desktop-titlebar-name" data-tauri-drag-region>Baize</span>
      <div className="desktop-titlebar-controls">
        <button type="button" onClick={() => void windowControls.minimize()} aria-label={tx("Minimize")} title={tx("Minimize")}>–</button>
        <button type="button" onClick={() => void windowControls.toggleMaximize()} aria-label={tx("Maximize")} title={tx("Maximize")}>▢</button>
        <button type="button" className="desktop-titlebar-close" onClick={() => void windowControls.close()} aria-label={tx("Close window")} title={tx("Close window")}>×</button>
      </div>
    </div>
  );
}
