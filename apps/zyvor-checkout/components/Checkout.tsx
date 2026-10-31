"use client";
import { useCallback, useEffect, useRef, useState } from "react";
import { useSearchParams } from "next/navigation";
import { LangPicker, useI18n } from "@/components/useI18n";
import { METHOD_KEY, VPA, failureKey, money, pollDelay, supportLink, supportRef, type Session } from "@/lib/checkout";
import type { Key } from "@/lib/i18n";

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

/** The support reference and, if the operator configured one, a report-a-problem link. */
function Help({ t, intent }: { t: (k: Key, v?: Record<string, string>) => string; intent: string }) {
  const ref = supportRef(intent);
  const href = supportLink(process.env.NEXT_PUBLIC_ZYVOR_SUPPORT_URL, ref);
  return (
    <p className="foot help">
      {t("supportRef", { ref })}
      {href && <> · <a href={href} rel="noopener noreferrer">{t("reportProblem")}</a></>}
    </p>
  );
}

export function Checkout({ tenant, intent }: { tenant: string; intent: string }) {
  const { lang, setLang, t } = useI18n();
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
      setError("net");
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

  const errText = error === "net" ? t("errNetwork") : error === "vpa" ? t("errVpa") : error ? t("errStart") : null;
  // With a provider-issued QR / intent the payer never types a UPI ID.
  const upiIntent = session?.upi_flow === "intent";
  const needsVpa = method === "upi" && !upiIntent;
  const vpaOk = VPA.test(vpa);
  const showVpaError = touched && needsVpa && !vpaOk;

  async function pay(e: React.FormEvent) {
    e.preventDefault();
    setTouched(true);
    setError(null);
    if (needsVpa && !vpaOk) return;
    setBusy(true);
    try {
      const r = await fetch(`${base}/attempts`, {
        method: "POST",
        headers: { "content-type": "application/json", "idempotency-key": idemKey(intent, method, vpa) },
        body: JSON.stringify({ secret, method, vpa: needsVpa ? vpa : undefined, upi_flow: method === "upi" && upiIntent ? "intent" : undefined }),
      });
      const body = await r.json().catch(() => ({}));
      if (!r.ok) {
        setError(body?.error?.code === "invalid_vpa" ? "vpa" : "start");
      } else {
        setRetrying(false);
        setWaited(0);
        await openHostedIfNeeded(body?.client_action);
        await load();
      }
    } catch {
      setError("start");
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
    return <div className="card state" role="alert"><h2>{t("invalidTitle")}</h2><p>{t("invalidBody")}</p><LangPicker lang={lang} setLang={setLang} label={t("language")} /></div>;
  }
  if (!session) {
    return <div className="card state" role="status" aria-live="polite">{errText ? <p>{errText}</p> : <><div className="spin" /><p>{t("loading")}</p></>}</div>;
  }

  const amount = money(session.amount_minor, session.currency);
  const head = (
    <>
      <p className="merchant">{session.merchant ?? t("payment")}</p>
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
          <h2>{t("paidTitle")}</h2>
          <p>{session.merchant ? t("paidTo", { amount, merchant: session.merchant }) : t("paidPlain", { amount })}</p>
        </div>
        <Help t={t} intent={intent} />
      </div>
    );
  }
  if (session.phase === "confirming") {
    return (
      <div className="card">
        {head}
        <div className="state" role="status" aria-live="polite">
          <div className="spin" />
          <h2>{t("confirmTitle")}</h2>
          <p>{t("confirmBody")}</p>
        </div>
        <Help t={t} intent={intent} />
      </div>
    );
  }
  if (session.phase === "awaiting_payment" && session.attempt) {
    const upi = session.attempt.action?.type === "upi_collect";
    const intentUri = session.attempt.action?.type === "upi_intent" ? String(session.attempt.action.uri ?? "") : "";
    const qrImage = session.attempt.action?.type === "upi_qr" ? String(session.attempt.action.image ?? "") : "";
    // A partner-rendered QR (desktop and counter screens): only a PNG data URL is ever shown.
    if (qrImage.startsWith("data:image/png;base64,")) {
      return (
        <div className="card">
          {head}
          <div className="state" role="status" aria-live="polite">
            <h2>{t("scanTitle")}</h2>
            <img className="qr" src={qrImage} width={220} height={220} alt={t("scanAlt")} />
            <p>{t("scanBody")}</p>
            {waited >= 60000 && <p className="notice warn">{t("stillWaiting")}</p>}
            <button className="link" type="button" onClick={() => void load()}>{t("checkStatus")}</button>
          </div>
          <Help t={t} intent={intent} />
        </div>
      );
    }
    // Only a UPI link or the partner's own https link is ever opened: never javascript:, data: or anything else.
    if (intentUri.startsWith("upi://") || intentUri.startsWith("https://")) {
      // A partner-issued payload, shown as-is. Neither the QR nor the app opening proves payment: the page
      // moves to "paid" only when the server reports a verified result.
      return (
        <div className="card">
          {head}
          <div className="state" role="status" aria-live="polite">
            <h2>{t("scanTitle")}</h2>
            <img className="qr" src={`${base}/upi-qr.svg?secret=${encodeURIComponent(secret)}`} width={220} height={220} alt={t("scanAlt")} />
            <p>{t("scanBody")}</p>
            <a className="pay" href={intentUri}>{t("openUpiApp")}</a>
            <p className="desc">{t("upiAppFail")}</p>
            {waited >= 60000 && <p className="notice warn">{t("stillWaiting")}</p>}
            <button className="link" type="button" onClick={() => void load()}>{t("checkStatus")}</button>
          </div>
          <Help t={t} intent={intent} />
        </div>
      );
    }
    return (
      <div className="card">
        {head}
        <div className="state" role="status" aria-live="polite">
          <div className="spin" />
          <h2>{upi ? t("approveTitle") : t("completeTitle")}</h2>
          <p>{upi ? t("approveBody") : t("completeBody")}</p>
          {waited >= 60000 && <p className="notice warn">{t("stillWaiting")}</p>}
          <button className="link" type="button" onClick={() => void load()}>{t("checkStatus")}</button>
        </div>
        <Help t={t} intent={intent} />
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
          {t(failureKey(session.attempt?.failure_reason ?? null))}
        </div>
      )}
      {(!failed) && (
        <>
          <fieldset className="methods">
            <legend>{t("methodLegend")}</legend>
            {session.methods.map((m) => (
              <label key={m}>
                <input type="radio" name="method" value={m} checked={method === m} onChange={() => setMethod(m)} />
                <span>{t(METHOD_KEY[m])}</span>
              </label>
            ))}
          </fieldset>
          {method === "upi" && upiIntent ? (
            <p className="desc">{t("upiIntentDesc")}</p>
          ) : method === "upi" ? (
            <div>
              <label className="f" htmlFor="vpa">{t("upiId")}</label>
              <input id="vpa" type="text" inputMode="email" autoComplete="off" autoCapitalize="off" spellCheck={false} placeholder="name@bank" value={vpa}
                onChange={(e) => setVpa(e.target.value.trim())} onBlur={() => setTouched(true)} aria-invalid={showVpaError} aria-describedby="vpa-err" />
              <p id="vpa-err" className="err" aria-live="polite">{showVpaError ? t("upiHint") : ""}</p>
            </div>
          ) : (
            <p className="desc">{t("hosted", { method: t(METHOD_KEY[method]) })}</p>
          )}
          {errText && <div className="notice bad" role="alert">{errText}</div>}
          <button className="pay" disabled={busy}>{busy ? t("starting") : t("pay", { amount })}</button>
        </>
      )}
      {failed && <button className="pay" type="button" onClick={() => { setRetrying(true); setError(null); }}>{t("tryAgain")}</button>}
      <p className="foot">{t("secured")}</p>
      <Help t={t} intent={intent} />
      <LangPicker lang={lang} setLang={setLang} label={t("language")} />
    </form>
  );
}
