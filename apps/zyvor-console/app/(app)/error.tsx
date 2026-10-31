"use client";

/** Shown inside the app shell when a page cannot load, so the menu and top bar stay usable. */
export default function ErrorPage({ error, reset }: { error: Error & { digest?: string }; reset: () => void }) {
  return (
    <div className="card" role="alert" style={{ maxWidth: 560 }}>
      <h1 style={{ fontSize: "1.5rem" }}>This page could not load</h1>
      <p className="lede">The server did not answer, or your role is not allowed to see this page. Your work was not lost.</p>
      <div style={{ display: "flex", gap: 10, marginTop: 16, flexWrap: "wrap" }}>
        <button className="btn primary" onClick={reset}>Try again</button>
        <a className="btn" href="/">Back to overview</a>
        <a className="btn" href="/login">Sign in again</a>
      </div>
      {error.digest && <p className="fine" style={{ marginTop: 14 }}>Reference: <span className="mono">{error.digest}</span></p>}
    </div>
  );
}
