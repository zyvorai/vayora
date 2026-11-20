// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

"use client";
import { useEffect, useMemo, useState } from "react";
import { LangPicker, useI18n } from "@/components/useI18n";
import { money } from "@/lib/checkout";
import { LOCALE, answersBody, formProblems, maxFor, orderErrorKey, seatCount, subtotal, tierNote, when, type PublicEvent } from "@/lib/events";
import { evt, type EvtKey } from "@/lib/events-i18n";

/** One idempotency key per distinct order, kept across reloads so a double tap never holds seats twice. */
function idemKey(sig: string): string {
  const k = `zyvor:evt-idem:${sig}`;
  try {
    return sessionStorage.getItem(k) ?? (sessionStorage.setItem(k, crypto.randomUUID()), sessionStorage.getItem(k)!);
  } catch {
    return crypto.randomUUID();
  }
}

/** The public page of an event: tiers with their sale windows, the registration form, and a button that holds the seats and goes to payment. */
export function EventPage({ tenant, event }: { tenant: string; event: string }) {
  const { lang, setLang, t } = useI18n();
  const e = (k: EvtKey, v?: Record<string, string>) => evt(lang, k, v);
  const base = `/api/zyvor-events/${tenant}`;
  const [ev, setEv] = useState<PublicEvent | null>(null);
  const [missing, setMissing] = useState(false);
  const [qty, setQty] = useState<Record<string, number>>({});
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [phone, setPhone] = useState("");
  const [coupon, setCoupon] = useState("");
  const [answers, setAnswers] = useState<Record<string, string | boolean>>({});
  const [tried, setTried] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch(`${base}/${event}`)
      .then((r) => (r.ok ? r.json() : Promise.reject()))
      .then(setEv)
      .catch(() => setMissing(true));
  }, [base, event]);

  const picker = <LangPicker lang={lang} setLang={setLang} label={e("language")} />;
  const locale = LOCALE[lang] ?? "en-IN";
  const problems = useMemo(() => (ev ? formProblems(ev.form_fields, answers) : {}), [ev, answers]);
  if (missing) return <div className="card state" role="alert"><h1>{e("eventGoneTitle")}</h1><p>{e("eventGoneBody")}</p>{picker}</div>;
  if (!ev) return <div className="card state" role="status"><div className="spin" /><p>{e("loadingEvent")}</p></div>;

  const total = subtotal(ev.tiers, qty);
  const seats = seatCount(qty);
  const nameBad = !name.trim();
  const emailBad = !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim());
  const formOk = !nameBad && !emailBad && Object.keys(problems).length === 0 && seats > 0;

  async function place() {
    if (!ev) return;
    setTried(true);
    setError(null);
    if (seats === 0) return setError("pickTickets");
    if (!formOk) return setError("fixForm");
    setBusy(true);
    const lines = Object.entries(qty).filter(([, n]) => n > 0).map(([tier_id, quantity]) => ({ tier_id, quantity }));
    const body = { buyer: { name: name.trim(), email: email.trim(), phone: phone.trim() || undefined }, lines, answers: answersBody(ev.form_fields, answers), coupon: coupon.trim() || undefined };
    try {
      const r = await fetch(`${base}/${event}/orders`, { method: "POST", headers: { "content-type": "application/json", "idempotency-key": idemKey(JSON.stringify([event, body])) }, body: JSON.stringify(body) });
      const j = await r.json();
      if (!r.ok) throw new Error(orderErrorKey(j?.error?.code));
      const q = `eo=${encodeURIComponent(j.order_id)}&et=${encodeURIComponent(j.order_token)}`;
      if (j.payment) window.location.assign(`/pay/${tenant}/${j.payment.intent_id}?s=${encodeURIComponent(j.payment.client_secret)}&${q}`);
      else window.location.assign(`${j.order_path}?t=${encodeURIComponent(j.order_token)}`);
    } catch (err) {
      setError(err instanceof Error && err.message.startsWith("err") ? err.message : "errGeneric");
      setBusy(false);
    }
  }

  const gone = ev.status === "cancelled";
  const closed = !ev.on_sale && !gone;
  return (
    <div className="card evt">
      <p className="merchant">{ev.merchant ?? e("eventTitle")}</p>
      <h1 className="evt-title">{ev.name}</h1>
      <p className="desc" style={{ marginBottom: 6 }}>{when(ev.starts_at, locale)}</p>
      <p className="desc" style={{ marginBottom: 14 }}>{ev.mode === "online" ? e("online") : ev.venue}</p>
      {ev.description && <p className="evt-about">{ev.description}</p>}
      {gone && <div className="notice warn" role="status"><strong>{e("cancelledTitle")}</strong><br />{e("cancelledBody")}</div>}
      {closed && <div className="notice warn" role="status">{e("salesClosed")}</div>}
      {!gone && (
        <>
          <h2 className="evt-h">{e("tickets")}</h2>
          <ul className="tiers">
            {ev.tiers.map((tr) => {
              const max = maxFor(tr);
              const note = tierNote(tr);
              const n = qty[tr.id] ?? 0;
              return (
                <li key={tr.id} className={max === 0 ? "tier off" : "tier"}>
                  <div className="tier-main">
                    <strong>{tr.name}</strong>
                    {tr.description && <span className="tier-d">{tr.description}</span>}
                    {tr.price_kind === "early_bird" && <span className="pill">{e("earlyBird")}</span>}
                    {note && <span className="tier-d" role="status">{note === "salesStart" ? e("salesStart", { when: tr.sale.starts_at ? when(tr.sale.starts_at, locale) : "" }) : note === "fewLeft" ? e("fewLeft", { n: String(tr.available) }) : e(note)}</span>}
                  </div>
                  <div className="tier-price">
                    <span>{tr.price_minor === 0 ? e("free") : money(tr.price_minor, ev.currency)}</span>
                    {tr.price_kind === "early_bird" && <s className="tier-d">{money(tr.regular_price_minor, ev.currency)}</s>}
                  </div>
                  {max > 0 && (
                    <div className="stepper" role="group" aria-label={e("quantityFor", { tier: tr.name })}>
                      <button type="button" aria-label={e("fewer")} disabled={n <= 0} onClick={() => setQty({ ...qty, [tr.id]: n - 1 })}>−</button>
                      <output aria-live="polite">{n}</output>
                      <button type="button" aria-label={e("more")} disabled={n >= max} onClick={() => setQty({ ...qty, [tr.id]: n + 1 })}>+</button>
                    </div>
                  )}
                </li>
              );
            })}
          </ul>

          {ev.on_sale && (
            <>
              <h2 className="evt-h">{e("yourDetails")}</h2>
              <label className="f" htmlFor="b-name">{e("name")}</label>
              <input id="b-name" type="text" value={name} onChange={(x) => setName(x.target.value)} autoComplete="name" aria-invalid={tried && nameBad} />
              <label className="f" htmlFor="b-mail" style={{ marginTop: 12 }}>{e("email")}</label>
              <input id="b-mail" type="text" inputMode="email" value={email} onChange={(x) => setEmail(x.target.value)} autoComplete="email" aria-invalid={tried && emailBad} aria-describedby="b-mail-h" />
              <p className="tier-d" id="b-mail-h" style={{ margin: "4px 2px 0" }}>{e("emailHint")}</p>
              <label className="f" htmlFor="b-phone" style={{ marginTop: 12 }}>{e("phone")}</label>
              <input id="b-phone" type="text" inputMode="tel" value={phone} onChange={(x) => setPhone(x.target.value)} autoComplete="tel" />
              {ev.form_fields.map((f) => (
                <div key={f.key} style={{ marginTop: 12 }}>
                  {f.kind === "checkbox" ? (
                    <label className="check">
                      <input type="checkbox" checked={answers[f.key] === true} onChange={(x) => setAnswers({ ...answers, [f.key]: x.target.checked })} aria-invalid={tried && !!problems[f.key]} />
                      <span>{f.label}{f.required ? ` (${e("required")})` : ""}</span>
                    </label>
                  ) : (
                    <>
                      <label className="f" htmlFor={`f-${f.key}`}>{f.label}{f.required ? ` (${e("required")})` : ""}</label>
                      {f.kind === "select" ? (
                        <select id={`f-${f.key}`} value={String(answers[f.key] ?? "")} onChange={(x) => setAnswers({ ...answers, [f.key]: x.target.value })} aria-invalid={tried && !!problems[f.key]}>
                          <option value="">{e("chooseOne")}</option>
                          {(f.options ?? []).map((o) => <option key={o} value={o}>{o}</option>)}
                        </select>
                      ) : (
                        <input id={`f-${f.key}`} type="text" inputMode={f.kind === "phone" ? "tel" : f.kind === "email" ? "email" : "text"} value={String(answers[f.key] ?? "")} onChange={(x) => setAnswers({ ...answers, [f.key]: x.target.value })} aria-invalid={tried && !!problems[f.key]} />
                      )}
                    </>
                  )}
                </div>
              ))}
              <label className="f" htmlFor="b-coupon" style={{ marginTop: 12 }}>{e("couponCode")}</label>
              <input id="b-coupon" type="text" value={coupon} onChange={(x) => setCoupon(x.target.value)} autoCapitalize="characters" spellCheck={false} autoComplete="off" aria-describedby="b-coupon-h" />
              <p className="tier-d" id="b-coupon-h" style={{ margin: "4px 2px 0" }}>{e("couponNote")}</p>

              {seats > 0 && (
                <dl className="rows" style={{ marginTop: 16 }}>
                  <dt>{e("subtotal")}</dt><dd>{money(total, ev.currency)}</dd>
                </dl>
              )}
              {error && <div className="notice bad" role="alert">{error === "pickTickets" || error === "fixForm" ? e(error) : e(error as EvtKey)}</div>}
              <button className="pay" onClick={place} disabled={busy}>{busy ? e("placing") : total === 0 ? e("getTickets") : e("placeOrder", { amount: money(total, ev.currency) })}</button>
              <p className="foot">{e("holdNote", { minutes: String(ev.hold_minutes) })}</p>
            </>
          )}
        </>
      )}
      <p className="foot">{t("secured")}</p>
      {picker}
    </div>
  );
}
