"use client";
import { useEffect, useState } from "react";
import { money } from "@/lib/checkout";

type Info = { merchant: string | null; description: string | null; amount_minor: number; discount_minor: number; fee_minor: number; total_minor: number; coupon_applied: boolean; currency: string; status: "active" | "expired" | "used_up" | "canceled" | "paid" };

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
  const [code, setCode] = useState("");
  const [couponMsg, setCouponMsg] = useState<string | null>(null);
  /** The code the price on screen was worked out with; what "Continue" sends. */
  const [appliedCode, setAppliedCode] = useState<string | null>(null);

  useEffect(() => {
    fetch(base)
      .then((r) => (r.ok ? r.json() : Promise.reject()))
      .then(setInfo)
      .catch(() => setMissing(true));
  }, [base]);

  async function apply() {
    setCouponMsg(null);
    const c = code.trim();
    const r = await fetch(c ? `${base}?coupon=${encodeURIComponent(c)}` : base);
    if (r.ok) {
      setInfo(await r.json());
      setAppliedCode(c || null);
      setCouponMsg(c ? "Coupon applied." : null);
    } else {
      // a code that does not work must not leave an old discount on screen
      const plain = await fetch(base);
      if (plain.ok) setInfo(await plain.json());
      setAppliedCode(null);
      setCouponMsg("That coupon is not valid.");
    }
  }

  async function start() {
    setBusy(true);
    setError(null);
    try {
      const r = await fetch(`${base}/start`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ coupon: appliedCode ?? undefined }) });
      const j = await r.json();
      if (!r.ok) throw new Error(j?.error?.message ?? "Could not start the payment.");
      window.location.assign(`/pay/${tenant}/${j.intent_id}?s=${encodeURIComponent(j.client_secret)}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not start the payment.");
      setBusy(false);
    }
  }

  if (missing) return <div className="card state" role="alert"><h2>This payment link isn't valid</h2><p>Please ask the merchant for a new link.</p></div>;
  if (!info) return <div className="card state" role="status"><div className="spin" /><p>Loading…</p></div>;
  const gone = GONE[info.status];
  const adjusted = info.discount_minor > 0 || info.fee_minor > 0;
  return (
    <div className="card">
      <p className="merchant">{info.merchant ?? "Payment"}</p>
      <h1 className="amount">{money(gone ? info.amount_minor : info.total_minor, info.currency)}</h1>
      <p className="desc">{info.description ?? " "}</p>
      {gone ? <p className="notice warn" role="status">{gone}</p> : (
        <>
          {adjusted && (
            <dl className="rows">
              <dt>Amount</dt><dd>{money(info.amount_minor, info.currency)}</dd>
              {info.discount_minor > 0 && <><dt>Coupon</dt><dd className="ok">−{money(info.discount_minor, info.currency)}</dd></>}
              {info.fee_minor > 0 && <><dt>Convenience fee</dt><dd>{money(info.fee_minor, info.currency)}</dd></>}
            </dl>
          )}
          <label className="f" htmlFor="coupon">Coupon code</label>
          <div className="couponrow">
            <input id="coupon" value={code} onChange={(e) => setCode(e.target.value)} autoCapitalize="characters" spellCheck={false} autoComplete="off" />
            <button type="button" className="link" onClick={apply} disabled={!code.trim()}>Apply</button>
          </div>
          {couponMsg && <p className={appliedCode ? "desc" : "err"} role="status" style={{ margin: "6px 2px 0", textAlign: "left" }}>{couponMsg}</p>}
          {error && <div className="notice bad" role="alert">{error}</div>}
          <button className="pay" onClick={start} disabled={busy}>{busy ? "Starting…" : `Continue to pay ${money(info.total_minor, info.currency)}`}</button>
        </>
      )}
      <p className="foot">Secured by Zyvor Payments</p>
    </div>
  );
}
