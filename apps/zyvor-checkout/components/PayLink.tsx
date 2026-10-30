"use client";
import { useEffect, useState } from "react";
import { LangPicker, useI18n } from "@/components/useI18n";
import { money } from "@/lib/checkout";
import type { Key } from "@/lib/i18n";

type Info = { merchant: string | null; description: string | null; amount_minor: number; discount_minor: number; fee_minor: number; total_minor: number; coupon_applied: boolean; currency: string; status: "active" | "expired" | "used_up" | "canceled" | "paid" };

const GONE: Record<string, Key> = { expired: "goneExpired", used_up: "goneUsed", canceled: "goneCanceled", paid: "gonePaid" };

/** Landing page for a payment link: shows what is being paid, then opens the payer's own checkout. */
export function PayLink({ tenant, token }: { tenant: string; token: string }) {
  const { lang, setLang, t } = useI18n();
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
      setCouponMsg(c ? "ok" : null);
    } else {
      // a code that does not work must not leave an old discount on screen
      const plain = await fetch(base);
      if (plain.ok) setInfo(await plain.json());
      setAppliedCode(null);
      setCouponMsg("bad");
    }
  }

  async function start() {
    setBusy(true);
    setError(null);
    try {
      const r = await fetch(`${base}/start`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ coupon: appliedCode ?? undefined }) });
      const j = await r.json();
      if (!r.ok) throw new Error(j?.error?.code === "coupon_invalid" ? "coupon" : "start");
      window.location.assign(`/pay/${tenant}/${j.intent_id}?s=${encodeURIComponent(j.client_secret)}`);
    } catch (e) {
      setError(e instanceof Error && e.message ? e.message : "start");
      setBusy(false);
    }
  }

  const picker = <LangPicker lang={lang} setLang={setLang} label={t("language")} />;
  if (missing) return <div className="card state" role="alert"><h2>{t("invalidTitle")}</h2><p>{t("invalidBody")}</p>{picker}</div>;
  if (!info) return <div className="card state" role="status"><div className="spin" /><p>{t("loadingShort")}</p></div>;
  const goneKey = GONE[info.status];
  const adjusted = info.discount_minor > 0 || info.fee_minor > 0;
  const errText = error === "coupon" ? t("couponBad") : error ? t("errStart") : null;
  return (
    <div className="card">
      <p className="merchant">{info.merchant ?? t("payment")}</p>
      <h1 className="amount">{money(goneKey ? info.amount_minor : info.total_minor, info.currency)}</h1>
      <p className="desc">{info.description ?? " "}</p>
      {goneKey ? <p className="notice warn" role="status">{t(goneKey)}</p> : (
        <>
          {adjusted && (
            <dl className="rows">
              <dt>{t("rowAmount")}</dt><dd>{money(info.amount_minor, info.currency)}</dd>
              {info.discount_minor > 0 && <><dt>{t("rowCoupon")}</dt><dd className="ok">−{money(info.discount_minor, info.currency)}</dd></>}
              {info.fee_minor > 0 && <><dt>{t("rowFee")}</dt><dd>{money(info.fee_minor, info.currency)}</dd></>}
            </dl>
          )}
          <label className="f" htmlFor="coupon">{t("couponCode")}</label>
          <div className="couponrow">
            <input id="coupon" value={code} onChange={(e) => setCode(e.target.value)} autoCapitalize="characters" spellCheck={false} autoComplete="off" />
            <button type="button" className="link" onClick={apply} disabled={!code.trim()}>{t("apply")}</button>
          </div>
          {couponMsg && <p className={appliedCode ? "desc" : "err"} role="status" style={{ margin: "6px 2px 0", textAlign: "left" }}>{couponMsg === "ok" ? t("couponOk") : t("couponBad")}</p>}
          {errText && <div className="notice bad" role="alert">{errText}</div>}
          <button className="pay" onClick={start} disabled={busy}>{busy ? t("starting") : t("continuePay", { amount: money(info.total_minor, info.currency) })}</button>
        </>
      )}
      <p className="foot">{t("secured")}</p>
      {picker}
    </div>
  );
}
