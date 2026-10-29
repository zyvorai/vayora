"use client";
import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";

const icon = (d: string): ReactNode => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={d} /></svg>
);
const items = [
  { href: "/", label: "Overview", icon: icon("M3 12l9-8 9 8M5 10v10h14V10") },
  { href: "/payments", label: "Payments", icon: icon("M3 7h18v10H3zM3 11h18") },
  { href: "/refunds", label: "Refunds", icon: icon("M9 14L4 9l5-5M4 9h10a6 6 0 010 12h-3") },
  { href: "/webhooks", label: "Webhooks", icon: icon("M4 12h4l2-6 4 12 2-6h4") },
  { href: "/developers", label: "Developers", icon: icon("M8 8l-5 4 5 4M16 8l5 4-5 4M14 5l-4 14") },
];

export function Nav() {
  const path = usePathname();
  return (
    <nav aria-label="Main">
      {items.map((i) => {
        const on = i.href === "/" ? path === "/" : path.startsWith(i.href);
        return (
          <Link key={i.href} href={i.href} className="nav" aria-current={on ? "page" : undefined}>
            {i.icon}{i.label}
          </Link>
        );
      })}
    </nav>
  );
}
