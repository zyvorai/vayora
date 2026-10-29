import type { ReactNode } from "react";
import "./globals.css";

export const metadata = { title: "Secure payment", robots: "noindex", referrer: "no-referrer" };
export const viewport = { width: "device-width", initialScale: 1, viewportFit: "cover" } as const;

export default function Root({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body><main>{children}</main></body>
    </html>
  );
}
