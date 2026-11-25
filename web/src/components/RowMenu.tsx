import { useEffect, useState } from "react";
import { MoreIcon } from "./icons";
import { tx } from "../i18n";

export interface MenuItem {
  label: string;
  onClick: () => void;
  danger?: boolean;
}

/** A "…" button that opens a short menu of actions for a row. Renders nothing without items. */
export function RowMenu({ items, label }: { items: MenuItem[]; label?: string }) {
  const [open, setOpen] = useState(false);
  // Escape closes the menu, as a click outside it does.
  useEffect(() => {
    if (!open) return;
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [open]);
  if (items.length === 0) return null;
  const title = label ?? tx("More");
  return (
    <div className="row-menu">
      <button
        type="button"
        className="row-menu-button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={title}
        title={title}
        onClick={() => setOpen((current) => !current)}
      >
        <MoreIcon />
      </button>
      {open && (
        <>
          <div className="plugin-menu-backdrop" onClick={() => setOpen(false)} />
          <div className="plugin-menu row-menu-list" role="menu">
            {items.map((item) => (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                className={item.danger ? "danger" : undefined}
                onClick={() => {
                  setOpen(false);
                  item.onClick();
                }}
              >
                {item.label}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
