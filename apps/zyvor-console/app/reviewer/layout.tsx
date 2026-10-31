import type { ReactNode } from "react";
import { ThemeToggle } from "@/components/Theme";
import { reviewerSignOut } from "@/lib/reviewer-actions";
import { requireReviewerSurface, reviewerKey } from "@/lib/reviewer";

export const dynamic = "force-dynamic";
export const metadata = { title: "Zyvor review", robots: { index: false, follow: false } };

export default async function ReviewerLayout({ children }: { children: ReactNode }) {
  requireReviewerSurface();
  const signedIn = !!(await reviewerKey());
  return (
    <div className="col" style={{ maxWidth: 980, margin: "0 auto", width: "100%" }}>
      <header className="top">
        <a className="brand" href="/reviewer"><img src="/favicon.svg" alt="" width="26" height="26" />Zyvor review</a>
        <span className="grow" />
        <ThemeToggle />
        {signedIn && <form action={reviewerSignOut}><button className="linkbtn" type="submit">Sign out</button></form>}
      </header>
      <main id="main">{children}</main>
    </div>
  );
}
