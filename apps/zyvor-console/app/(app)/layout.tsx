import type { ReactNode } from "react";
import { CommandPalette, PaletteButton } from "@/components/CommandPalette";
import { Nav } from "@/components/Nav";
import { ThemeToggle } from "@/components/Theme";
import { logout } from "@/lib/actions";

export const dynamic = "force-dynamic";

export default function AppLayout({ children }: { children: ReactNode }) {
  return (
    <div className="shell">
      <aside className="side">
        <a className="brand" href="/"><img src="/favicon.svg" alt="" width="26" height="26" />Zyvor</a>
        <PaletteButton />
        <Nav />
        <div className="spacer" />
        <div className="foot">
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
