"use client";
import { useEffect, useState } from "react";
import { money } from "@/lib/checkout";

type Info = { merchant: string | null; description: string | null; amount_minor: number; currency: string; status: "active" | "expired" | "used_up" | "canceled" | "paid" };

const GONE: Record<string, string> = {
  expired: "This payment link has expired.",
  used_up: "This payment link has reached its limit.",
  canceled: "This payment link is no longer active.",
  paid: "Nothing is due. This has already been paid.",
};

/** Landing page for a payment link: shows what is being paid, then opens the payer's own checkout. */
export function PayLink({ tenant, token }: { tenant: string; token: string }) {
  const base = `/api/zyvor/links/${tenant}/${token}`;
  const [info, setInfo] = useState<Info | null>(null);
  const [missing, setMissing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch(base)
      .then((r) => (r.ok ? r.json() : Promise.reject()))
      .then(setInfo)
      .catch(() => setMissing(true));
  }, [base]);

  async function start() {
    setBusy(true);
    setError(null);
    try {
      const r = await fetch(`${base}/start`, { method: "POST" });
      const j = await r.json();
      if (!r.ok) throw new Error(j?.error?.message ?? "Could not start the payment.");
      window.location.assign(`/pay/${tenant}/${j.intent_id}?s=${encodeURIComponent(j.client_secret)}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not start the payment.");
      setBusy(false);
    }
  }

  if (missing) return <div className="card state" role="alert"><h1>Link not found</h1><p>Check the link you were sent.</p></div>;
  if (!info) return <div className="card state" role="status"><div className="spin" /><p>Loading…</p></div>;
  const gone = GONE[info.status];
  return (
    <div className="card">
      {info.merchant && <p className="muted">{info.merchant}</p>}
      <h1>{info.description ?? "Payment"}</h1>
      {gone ? <p role="status">{gone}</p> : (
        <>
          <p className="amount">{money(info.amount_minor, info.currency)}</p>
          <button className="btn primary" onClick={start} disabled={busy}>{busy ? "Starting…" : "Continue to pay"}</button>
        </>
      )}
      {error && <p className="note bad" role="alert">{error}</p>}
    </div>
  );
}
