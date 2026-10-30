import type { ReactNode } from "react";
import { CommandPalette, PaletteButton } from "@/components/CommandPalette";
import { Nav } from "@/components/Nav";
import { ThemeToggle } from "@/components/Theme";
import { logout } from "@/lib/actions";
import { currentSession } from "@/lib/auth";

export const dynamic = "force-dynamic";

export default async function AppLayout({ children }: { children: ReactNode }) {
  const me = await currentSession();
  return (
    <div className="shell">
      <aside className="side">
        <a className="brand" href="/"><img src="/favicon.svg" alt="" width="26" height="26" />Zyvor</a>
        <PaletteButton />
        <Nav role={me?.r ?? "viewer"} />
        <div className="spacer" />
        <div className="foot">
          {me && <p className="fine" style={{ margin: "0 0 8px" }}>{me.u} · {me.r}</p>}
          <span style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
            <form action={logout}><button className="linkbtn" type="submit">Sign out</button></form>
            <ThemeToggle />
          </span>
        </div>
      </aside>
      <main id="main">{children}</main>
      <CommandPalette />
    </div>
  );
}
