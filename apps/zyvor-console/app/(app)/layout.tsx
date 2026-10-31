import type { ReactNode } from "react";
import { CommandPalette, PaletteButton } from "@/components/CommandPalette";
import { EnvSwitch } from "@/components/EnvSwitch";
import { Nav } from "@/components/Nav";
import { ThemeToggle } from "@/components/Theme";
import { logout } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { api } from "@/lib/zyvor";

export const dynamic = "force-dynamic";

export default async function AppLayout({ children }: { children: ReactNode }) {
  const me = await currentSession();
  // live mode taken away by Zyvor: say so on every page, not only on Onboarding (the API refuses the writes either way)
  const paused = me?.e === "live" && (await (await api()).liveAccess.get().catch(() => null))?.status === "revoked";
  return (
    <div className="shell">
      <aside className="side">
        <a className="brand" href="/"><img src="/favicon.svg" alt="" width="26" height="26" />Zyvor</a>
        <Nav role={me?.r ?? "viewer"} env={me?.e} />
      </aside>
      <div className="col">
        <header className="top">
          <PaletteButton />
          <span className="grow" />
          {me?.e && me.r !== "cashier" && <EnvSwitch env={me.e} />}
          <ThemeToggle />
          {me && (
            <details className="menu">
              <summary aria-label="Account menu"><span className="avatar" aria-hidden="true">{me.u.slice(0, 1).toUpperCase()}</span></summary>
              <div className="pop">
                <p className="fine" style={{ margin: "0 0 8px" }}>{me.u} · {me.r}</p>
                <a href="/account">Account and security</a>
                <form action={logout}><button className="linkbtn" type="submit">Sign out</button></form>
              </div>
            </details>
          )}
        </header>
        {paused && <div className="testbar bad" role="alert">Zyvor has paused live mode for this business: new payments and payouts are stopped. <a href="/onboarding">See why</a></div>}
        {me?.e === "test" && <div className="testbar" role="status">Test mode: nothing here moves real money.</div>}
        <main id="main">{children}</main>
      </div>
      <CommandPalette />
    </div>
  );
}
