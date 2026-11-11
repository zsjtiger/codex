import { windowControls } from "../desktop";
import { tx } from "../i18n";

/** The desktop window's close, minimize and maximize buttons, shown at the top of the sidebar. */
export function WindowLights() {
  return (
    <div className="window-lights">
      <button type="button" className="light light-close" onClick={() => void windowControls.close()} aria-label={tx("Close window")} title={tx("Close window")} />
      <button type="button" className="light light-min" onClick={() => void windowControls.minimize()} aria-label={tx("Minimize")} title={tx("Minimize")} />
      <button type="button" className="light light-max" onClick={() => void windowControls.toggleMaximize()} aria-label={tx("Maximize")} title={tx("Maximize")} />
    </div>
  );
}
