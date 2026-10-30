import type { ReactNode } from "react";
import "./globals.css";

export const metadata = { title: "Zyvor Console", icons: { icon: "/favicon.svg" } };

const themeScript = `try{var t=localStorage.getItem("zyvor-theme")}catch(e){}document.documentElement.dataset.theme=t||(matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light")`;

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head><script dangerouslySetInnerHTML={{ __html: themeScript }} /></head>
      <body>{children}</body>
    </html>
  );
}
