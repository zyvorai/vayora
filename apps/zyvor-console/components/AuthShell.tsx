import type { ReactNode } from "react";

/** The centred card used by the pages people see before they are signed in. */
export function AuthShell({ title, lede, children }: { title: string; lede?: string; children: ReactNode }) {
  return (
    <div className="login">
      <div className="aurora" aria-hidden="true" />
      <div className="card">
        <img src="/favicon.svg" alt="" width="44" height="44" style={{ borderRadius: 11 }} />
        <h1>{title}</h1>
        {lede && <p className="lede">{lede}</p>}
        {children}
      </div>
    </div>
  );
}
