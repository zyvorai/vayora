"use client";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useRef, useState } from "react";

const ITEMS = [
  { label: "Overview", hint: "Go to", href: "/", k: "home dashboard" },
  { label: "Payments", hint: "Go to", href: "/payments", k: "intents transactions" },
  { label: "Failed payments", hint: "Filter", href: "/payments?status=failed", k: "declined errors" },
  { label: "Refunds", hint: "Go to", href: "/refunds", k: "approve returns" },
  { label: "Disputes", hint: "Go to", href: "/disputes", k: "chargebacks evidence" },
  { label: "Reconciliation", hint: "Go to", href: "/reconciliation", k: "settlements cases bank" },
  { label: "Collections", hint: "Go to", href: "/collections", k: "ageing overdue promises reminders" },
  { label: "Billing", hint: "Go to", href: "/billing", k: "invoices subscriptions plans milestones" },
  { label: "Webhooks", hint: "Go to", href: "/webhooks", k: "events replay outbox" },
];

/** ⌘K / Ctrl-K jump menu: keyboard-first, every item is a plain link. */
export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const [i, setI] = useState(0);
  const router = useRouter();
  const input = useRef<HTMLInputElement>(null);
  const shown = useMemo(() => {
    const t = q.trim().toLowerCase();
    return ITEMS.filter((x) => !t || `${x.label} ${x.k}`.toLowerCase().includes(t));
  }, [q]);

  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); setOpen((o) => !o); setQ(""); setI(0); }
      else if (e.key === "Escape") setOpen(false);
    };
    const openIt = () => { setOpen(true); setQ(""); setI(0); };
    window.addEventListener("keydown", on);
    window.addEventListener("zyvor:palette", openIt);
    return () => { window.removeEventListener("keydown", on); window.removeEventListener("zyvor:palette", openIt); };
  }, []);
  useEffect(() => { if (open) input.current?.focus(); }, [open]);

  const go = (href: string) => { setOpen(false); router.push(href); };
  if (!open) return null;
  return (
    <div className="pal-back" onMouseDown={(e) => e.target === e.currentTarget && setOpen(false)}>
      <div className="pal" role="dialog" aria-modal="true" aria-label="Command menu">
        <input
          ref={input} value={q} placeholder="Jump to…" aria-label="Search commands" autoComplete="off" spellCheck={false}
          onChange={(e) => { setQ(e.target.value); setI(0); }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") { e.preventDefault(); setI((n) => Math.min(n + 1, shown.length - 1)); }
            else if (e.key === "ArrowUp") { e.preventDefault(); setI((n) => Math.max(n - 1, 0)); }
            else if (e.key === "Enter" && shown[i]) go(shown[i].href);
          }}
        />
        {shown.length === 0 ? <div className="none">Nothing matches “{q}”.</div> : (
          <ul role="listbox">
            {shown.map((x, n) => (
              <li key={x.href} role="option" aria-selected={n === i}>
                <button aria-selected={n === i} onMouseEnter={() => setI(n)} onClick={() => go(x.href)}>{x.label}<small>{x.hint}</small></button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

export function PaletteButton() {
  return (
    <button className="kbtn" type="button" onClick={() => window.dispatchEvent(new Event("zyvor:palette"))} aria-label="Open command menu">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7" /><path d="M20 20l-3.5-3.5" /></svg>
      Search<kbd>⌘K</kbd>
    </button>
  );
}
