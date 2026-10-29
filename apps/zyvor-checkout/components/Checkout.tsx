"use client";
import { useCallback, useEffect, useRef, useState } from "react";
import { useSearchParams } from "next/navigation";
import { METHOD_LABEL, VPA, failureCopy, money, pollDelay, type Session } from "@/lib/checkout";

type Method = Session["methods"][number];

/** One idempotency key per (intent, method, vpa), kept across reloads so a double tap never pays twice. */
function idemKey(intent: string, method: string, vpa: string): string {
  const k = `zyvor:idem:${intent}:${method}:${vpa}`;
  try {
    return sessionStorage.getItem(k) ?? (sessionStorage.setItem(k, crypto.randomUUID()), sessionStorage.getItem(k)!);
  } catch {
    return crypto.randomUUID();
  }
}

function loadScript(src: string): Promise<void> {
  return new Promise((res, rej) => {
    if (document.querySelector(`script[src="${src}"]`)) return res();
    const s = document.createElement("script");
    s.src = src;
    s.onload = () => res();
    s.onerror = () => rej(new Error("script"));
    document.head.appendChild(s);
  });
}

export function Checkout({ tenant, intent }: { tenant: string; intent: string }) {
  const secret = useSearchParams().get("s") ?? "";
  const base = `/api/zyvor/${tenant}/${intent}`;
  const [session, setSession] = useState<Session | null>(null);
  const [gone, setGone] = useState(false);
  const [method, setMethod] = useState<Method>("upi");
  const [vpa, setVpa] = useState("");
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [retrying, setRetrying] = useState(false);
  const [waited, setWaited] = useState(0);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const load = useCallback(async () => {
    try {
      const r = await fetch(`${base}?secret=${encodeURIComponent(secret)}`, { cache: "no-store" });
      if (r.status === 404) return setGone(true);
      if (!r.ok) throw new Error();
      const s: Session = await r.json();
      // UPI first for Indian customers, then the rest in a stable order.
      const order = ["upi", "card", "netbanking", "wallet"];
      s.methods = [...s.methods].sort((a, b) => order.indexOf(a) - order.indexOf(b));
      setSession(s);
      setMethod((m) => (s.methods.includes(m) ? m : s.methods[0] ?? "upi"));
    } catch {
      setError("We couldn't reach the payment service. Check your connection; this page will keep trying.");
    }
  }, [base, secret]);

  useEffect(() => { void load(); }, [load]);

  // Resumable: poll only while a payment is in flight.
  useEffect(() => {
    const d = session ? pollDelay(session.phase) : null;
    if (d === null) return;
    timer.current = setTimeout(() => { setWaited((w) => w + d); void load(); }, d);
    return () => { if (timer.current) clearTimeout(timer.current); };
  }, [session, load]);

  const vpaOk = VPA.test(vpa);
  const showVpaError = touched && method === "upi" && !vpaOk;

  async function pay(e: React.FormEvent) {
    e.preventDefault();
    setTouched(true);
    setError(null);
    if (method === "upi" && !vpaOk) return;
    setBusy(true);
    try {
      const r = await fetch(`${base}/attempts`, {
        method: "POST",
        headers: { "content-type": "application/json", "idempotency-key": idemKey(intent, method, vpa) },
        body: JSON.stringify({ secret, method, vpa: method === "upi" ? vpa : undefined }),
      });
      const body = await r.json().catch(() => ({}));
      if (!r.ok) {
        setError(body?.error?.code === "invalid_vpa" ? "That UPI ID doesn't look right. It should look like name@bank." : "We couldn't start the payment. Please try again.");
      } else {
        setRetrying(false);
        setWaited(0);
        await openHostedIfNeeded(body?.client_action);
        await load();
      }
    } catch {
      setError("We couldn't start the payment. Please try again.");
    } finally {
      setBusy(false);
    }
  }

  /** Card, net banking and wallet details are collected by the provider's own hosted fields, never by us.
   *  NOTE: the provider script paths below are untested against live provider sandboxes. */
  async function openHostedIfNeeded(action: { type: string; params?: Record<string, unknown> } | null | undefined) {
    if (action?.type !== "hosted_checkout" || !action.params) return;
    const p = action.params as Record<string, string | number>;
    if (p.provider === "razorpay") {
      await loadScript("https://checkout.razorpay.com/v1/checkout.js");
      // @ts-expect-error provider global
      new window.Razorpay({ key: p.key, order_id: p.order_id, amount: p.amount, currency: p.currency, handler: () => void load() }).open();
    } else if (p.provider === "cashfree") {
      await loadScript("https://sdk.cashfree.com/js/v3/cashfree.js");
      // @ts-expect-error provider global
      await window.Cashfree({ mode: process.env.NEXT_PUBLIC_ZYVOR_ENV === "live" ? "production" : "sandbox" }).checkout({ paymentSessionId: p.payment_session_id });
    }
  }

  if (gone) {
    return <div className="card state" role="alert"><h2>This payment link isn't valid</h2><p>It may have expired or been mistyped. Please ask the merchant for a new link.</p></div>;
  }
  if (!session) {
    return <div className="card state" role="status" aria-live="polite">{error ? <p>{error}</p> : <><div className="spin" /><p>Loading your payment…</p></>}</div>;
  }

  const amount = money(session.amount_minor, session.currency);
  const head = (
    <>
      <p className="merchant">{session.merchant ?? "Payment"}</p>
      <h1 className="amount">{amount}</h1>
      <p className="desc">{session.description ?? " "}</p>
    </>
  );

  if (session.phase === "paid") {
    return (
      <div className="card">
        {head}
        <div className="state" role="status">
          <svg className="badge" viewBox="0 0 84 84" aria-hidden="true" stroke="var(--ok)"><circle cx="42" cy="42" r="40" /><path d="M26 43l11 11 21-23" /></svg>
          <h2>Payment successful</h2>
          <p>{amount} paid{session.merchant ? ` to ${session.merchant}` : ""}. You can close this page.</p>
        </div>
      </div>
    );
  }
  if (session.phase === "confirming") {
    return (
      <div className="card">
        {head}
        <div className="state" role="status" aria-live="polite">
          <div className="spin" />
          <h2>Confirming your payment</h2>
          <p>Your bank is taking longer than usual to confirm. <b>Please don't pay again.</b> This page will update by itself, and the merchant is notified as soon as it's confirmed.</p>
        </div>
      </div>
    );
  }
  if (session.phase === "awaiting_payment" && session.attempt) {
    const upi = session.attempt.action?.type === "upi_collect";
    return (
      <div className="card">
        {head}
        <div className="state" role="status" aria-live="polite">
          <div className="spin" />
          <h2>{upi ? "Approve in your UPI app" : "Complete your payment"}</h2>
          <p>{upi ? "We've sent a request to your UPI app. Open it and approve to finish." : "Finish the payment in the secure window. This page updates automatically."}</p>
          {waited >= 60000 && <p className="notice warn">Still waiting. If you've already approved, this can take a minute. If not, check your UPI app notifications.</p>}
          <button className="link" type="button" onClick={() => void load()}>Check status</button>
        </div>
      </div>
    );
  }

  // choose_method, or failed (shown with a way to retry)
  const failed = session.phase === "failed" && !retrying;
  return (
    <form className="card" onSubmit={pay} noValidate>
      {head}
      {failed && (
        <div className="notice bad" role="alert">
          {failureCopy(session.attempt?.failure_reason ?? null)}
        </div>
      )}
      {(!failed) && (
        <>
          <fieldset className="methods">
            <legend>Payment method</legend>
            {session.methods.map((m) => (
              <label key={m}>
                <input type="radio" name="method" value={m} checked={method === m} onChange={() => setMethod(m)} />
                <span>{METHOD_LABEL[m]}</span>
              </label>
            ))}
          </fieldset>
          {method === "upi" ? (
            <div>
              <label className="f" htmlFor="vpa">UPI ID</label>
              <input id="vpa" type="text" inputMode="email" autoComplete="off" autoCapitalize="off" spellCheck={false} placeholder="name@bank" value={vpa}
                onChange={(e) => setVpa(e.target.value.trim())} onBlur={() => setTouched(true)} aria-invalid={showVpaError} aria-describedby="vpa-err" />
              <p id="vpa-err" className="err" aria-live="polite">{showVpaError ? "Enter a UPI ID like name@bank." : ""}</p>
            </div>
          ) : (
            <p className="desc">You'll finish on {METHOD_LABEL[method]}'s secure page. We never see or store your details.</p>
          )}
          {error && <div className="notice bad" role="alert">{error}</div>}
          <button className="pay" disabled={busy}>{busy ? "Starting…" : `Pay ${amount}`}</button>
        </>
      )}
      {failed && <button className="pay" type="button" onClick={() => { setRetrying(true); setError(null); }}>Try again</button>}
      <p className="foot">Secured by Zyvor Payments</p>
    </form>
  );
}
