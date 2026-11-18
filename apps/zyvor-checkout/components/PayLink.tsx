"use client";
import { useEffect, useRef, useState } from "react";
import { LangPicker, useI18n } from "@/components/useI18n";
import { cardProps, money, safeHttps, type Brand } from "@/lib/checkout";
import type { Key } from "@/lib/i18n";

type Partial = { total_minor: number; paid_minor: number; balance_minor: number; available_minor: number; min_part_minor: number | null };
type Info = { merchant: string | null; description: string | null; message?: string | null; brand?: Brand | null; partial?: Partial | null; open_amount?: boolean; min_amount_minor?: number | null; max_amount_minor?: number | null; amount_minor: number; discount_minor: number; fee_minor: number; total_minor: number; coupon_applied: boolean; currency: string; status: "active" | "expired" | "used_up" | "canceled" | "paid" | "in_progress" };

const GONE: Record<string, Key> = { expired: "goneExpired", used_up: "goneUsed", canceled: "goneCanceled", paid: "gonePaid", in_progress: "goneBusy" };
/** Server error codes the payer can act on, mapped to the words for them. */
const START_ERR: Record<string, string> = { coupon_invalid: "coupon", link_busy: "busy", amount_out_of_range: "amount", link_unavailable: "gone" };

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
  /** What the payer typed on an open-amount link, in rupees. */
  const [typed, setTyped] = useState("");
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

  const part = info?.partial ?? null;
  /** A link the payer types an amount on: open-amount, or paid in parts (then the whole balance is filled in). */
  const typing = !!info?.open_amount || !!part;
  const prefilled = useRef(false);
  useEffect(() => {
    // fill in the whole balance once, so paying it all is one tap; after that the payer's own edits stand
    if (part && !prefilled.current) {
      prefilled.current = true;
      setTyped((part.available_minor / 100).toFixed(2));
    }
  }, [part]);
  const chosenMinor = Math.round(Number(typed.replace(/,/g, "")) * 100);
  const lo = part ? Math.min(part.min_part_minor ?? 1, part.available_minor) : (info?.min_amount_minor ?? 1);
  const hi = part ? part.available_minor : (info?.max_amount_minor ?? 1_000_000_000_000);
  const amountOk = Number.isFinite(chosenMinor) && chosenMinor >= lo && chosenMinor <= hi;

  async function start() {
    setBusy(true);
    setError(null);
    try {
      const r = await fetch(`${base}/start`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ coupon: appliedCode ?? undefined, amount_minor: typing ? chosenMinor : undefined }) });
      const j = await r.json();
      if (!r.ok) throw new Error(START_ERR[j?.error?.code as string] ?? "start");
      window.location.assign(`/pay/${tenant}/${j.intent_id}?s=${encodeURIComponent(j.client_secret)}`);
    } catch (e) {
      setError(e instanceof Error && e.message ? e.message : "start");
      setBusy(false);
    }
  }

  const picker = <LangPicker lang={lang} setLang={setLang} label={t("language")} />;
  if (missing) return <div className="card state" role="alert"><h1>{t("invalidTitle")}</h1><p>{t("invalidBody")}</p>{picker}</div>;
  if (!info) return <div className="card state" role="status"><div className="spin" /><p>{t("loadingShort")}</p></div>;
  const goneKey = GONE[info.status];
  const adjusted = info.discount_minor > 0 || info.fee_minor > 0;
  const errText = error === "coupon" ? t("couponBad") : error === "busy" ? t("errBusy") : error === "amount" ? t("errAmount") : error === "gone" ? t("errGone") : error ? t("errStart") : null;
  const logo = safeHttps(info.brand?.logo_url);
  const card = cardProps(info.brand) as React.HTMLAttributes<HTMLElement>;
  const rangeErr = typing && !!typed && !amountOk;
  return (
    <div {...card}>
      {logo && <img className="brandlogo" src={logo} alt="" height={40} referrerPolicy="no-referrer" onError={(e) => { e.currentTarget.style.display = "none"; }} />}
      <p className="merchant">{info.merchant ?? t("payment")}</p>
      {typing && !goneKey ? (
        <h1 className="amount">{amountOk ? money(chosenMinor, info.currency) : money(0, info.currency)}</h1>
      ) : (
        <h1 className="amount">{money(goneKey ? info.amount_minor : info.total_minor, info.currency)}</h1>
      )}
      <p className="desc">{info.description ?? " "}</p>
      {info.message && !goneKey && <p className="note">{info.message}</p>}
      {goneKey ? (
        <div className="notice warn" role="status">
          <p style={{ margin: 0 }}><strong>{t(goneKey)}</strong></p>
          {info.status !== "paid" && <p style={{ margin: "6px 0 0" }}>{info.merchant ? t("goneAsk", { merchant: info.merchant }) : t("goneAskPlain")}</p>}
        </div>
      ) : (
        <>
          {part && (
            <dl className="rows">
              <dt>{t("partialPaid")}</dt><dd>{money(part.paid_minor, info.currency)}</dd>
              <dt>{t("partialLeft")}</dt><dd>{money(part.balance_minor, info.currency)}</dd>
            </dl>
          )}
          {adjusted && (
            <dl className="rows">
              <dt>{t("rowAmount")}</dt><dd>{money(info.amount_minor, info.currency)}</dd>
              {info.discount_minor > 0 && <><dt>{t("rowCoupon")}</dt><dd className="ok">−{money(info.discount_minor, info.currency)}</dd></>}
              {info.fee_minor > 0 && <><dt>{t("rowFee")}</dt><dd>{money(info.fee_minor, info.currency)}</dd></>}
            </dl>
          )}
          {typing && (
            <>
              <label className="f" htmlFor="amt">{t("enterAmount")}</label>
              <input id="amt" type="text" value={typed} onChange={(e) => setTyped(e.target.value)} inputMode="decimal" autoComplete="off" aria-invalid={rangeErr} aria-describedby="amt-help" />
              <p id="amt-help" className={rangeErr ? "err" : "desc"} role={rangeErr ? "alert" : undefined} style={{ margin: "6px 2px 0", textAlign: "left" }}>
                {rangeErr ? t("amountRange", { min: money(lo, info.currency), max: money(hi, info.currency) }) : part ? t("partialHint", { min: money(lo, info.currency), max: money(hi, info.currency) }) : ""}
              </p>
            </>
          )}
          {!typing && <label className="f" htmlFor="coupon">{t("couponCode")}</label>}
          {!typing && <div className="couponrow">
            <input id="coupon" value={code} onChange={(e) => setCode(e.target.value)} autoCapitalize="characters" spellCheck={false} autoComplete="off" />
            <button type="button" className="link" onClick={apply} disabled={!code.trim()}>{t("apply")}</button>
          </div>}
          {couponMsg && <p className={appliedCode ? "desc" : "err"} role="status" style={{ margin: "6px 2px 0", textAlign: "left" }}>{couponMsg === "ok" ? t("couponOk") : t("couponBad")}</p>}
          {errText && <div className="notice bad" role="alert">{errText}</div>}
          <button className="pay" onClick={start} disabled={busy || (typing && !amountOk)}>{busy ? t("starting") : t("continuePay", { amount: money(typing ? (amountOk ? chosenMinor : 0) : info.total_minor, info.currency) })}</button>
        </>
      )}
      <p className="foot">{t("secured")}</p>
      {picker}
    </div>
  );
}
