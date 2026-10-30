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
  { href: "/links", label: "Payment links", icon: icon("M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1") },
  { href: "/payouts", label: "Payouts", icon: icon("M12 21V8M7 12l5-5 5 5M4 4h16") },
  { href: "/refunds", label: "Refunds", icon: icon("M9 14L4 9l5-5M4 9h10a6 6 0 010 12h-3") },
  { href: "/disputes", label: "Disputes", icon: icon("M12 3l9 16H3zM12 10v4M12 17v.01") },
  { href: "/reconciliation", label: "Reconciliation", icon: icon("M4 6h16M4 12h10M4 18h6M17 15l2 2 3-4") },
  { href: "/collections", label: "Collections", icon: icon("M12 3v18M17 7H9.5a3.5 3.5 0 000 7h5a3.5 3.5 0 010 7H6") },
  { href: "/billing", label: "Billing", icon: icon("M6 3h12v18l-3-2-3 2-3-2-3 2zM9 8h6M9 12h6") },
  { href: "/webhooks", label: "Webhooks", icon: icon("M4 12h4l2-6 4 12 2-6h4") },
];
const admin = [{ href: "/users", label: "People", icon: icon("M16 11a4 4 0 10-8 0 4 4 0 008 0zM4 21a8 8 0 0116 0") }];
const account = { href: "/account", label: "Account", icon: icon("M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6zM9 12l2 2 4-4") };

export function Nav({ role }: { role: "admin" | "finance" | "viewer" }) {
  const path = usePathname();
  return (
    <nav aria-label="Main">
      {[...items, ...(role === "admin" ? admin : []), account].map((i) => {
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
