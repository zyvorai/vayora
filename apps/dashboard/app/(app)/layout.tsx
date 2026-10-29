import type { ReactNode } from "react";
import { Nav } from "@/components/Nav";
import { ThemeToggle } from "@/components/Theme";
import { logout } from "@/lib/actions";

export const dynamic = "force-dynamic";

export default function AppLayout({ children }: { children: ReactNode }) {
  return (
    <div className="shell">
      <aside className="side">
        <a className="brand" href="/"><img src="/favicon.svg" alt="" width="26" height="26" />Vayora</a>
        <Nav />
        <div className="spacer" />
        <div className="foot">
          <span><span className="chip warn">Simulated UPI</span></span>
          <span style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
            <form action={logout}><button className="linkbtn" type="submit">Sign out</button></form>
            <ThemeToggle />
          </span>
        </div>
      </aside>
      <main id="main">{children}</main>
    </div>
  );
}
