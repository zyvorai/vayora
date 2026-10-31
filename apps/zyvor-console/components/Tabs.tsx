import Link from "next/link";

/** Tabs that are plain links (`?tab=`), so the page stays server-rendered and the back button works. */
export function Tabs({ base, current, tabs }: { base: string; current: string; tabs: { key: string; label: string; count?: number }[] }) {
  return (
    <nav className="tabs" aria-label="Sections">
      <div className="seg">
        {tabs.map((t) => <Link key={t.key} href={`${base}?tab=${t.key}`} aria-current={t.key === current}>{t.label}{t.count !== undefined && <span className="muted"> {t.count}</span>}</Link>)}
      </div>
    </nav>
  );
}
