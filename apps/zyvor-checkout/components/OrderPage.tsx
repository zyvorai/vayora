// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

"use client";
import { useCallback, useEffect, useRef, useState } from "react";
import { useSearchParams } from "next/navigation";
import { LangPicker, useI18n } from "@/components/useI18n";
import { money } from "@/lib/checkout";
import { LOCALE, when, type OrderView } from "@/lib/events";
import { evt, type EvtKey } from "@/lib/events-i18n";

const TITLE: Record<OrderView["status"], [EvtKey, EvtKey]> = {
  pending: ["orderPendingTitle", "orderPendingBody"],
  paid: ["orderPaidTitle", "orderPaidBody"],
  expired: ["orderExpiredTitle", "orderExpiredBody"],
  cancelled: ["orderCancelledTitle", "orderCancelledBody"],
  refunded: ["orderRefundedTitle", "orderRefundedBody"],
  needs_refund: ["orderNeedsRefundTitle", "orderNeedsRefundBody"],
};
const TICKET: Record<string, EvtKey> = { issued: "statusIssued", checked_in: "statusCheckedIn", refunded: "statusRefunded", void: "statusVoid" };

/** The buyer's order: while unpaid it waits for the payment; once paid it shows each ticket with its QR code. */
export function OrderPage({ tenant, order }: { tenant: string; order: string }) {
  const { lang, setLang, t } = useI18n();
  const e = (k: EvtKey, v?: Record<string, string>) => evt(lang, k, v);
  const token = useSearchParams().get("t") ?? "";
  const base = `/api/zyvor-events/${tenant}/orders/${order}`;
  const [view, setView] = useState<OrderView | null>(null);
  const [missing, setMissing] = useState(false);
  const [busy, setBusy] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const load = useCallback(async () => {
    try {
      const r = await fetch(`${base}?t=${encodeURIComponent(token)}`, { cache: "no-store" });
      if (!r.ok) return setMissing(true);
      const v: OrderView = await r.json();
      setView(v);
      // an unpaid order is re-checked until it settles
      if (v.status === "pending" || v.status === "needs_refund") timer.current = setTimeout(load, 4000);
    } catch {
      timer.current = setTimeout(load, 6000);
    }
  }, [base, token]);

  useEffect(() => {
    load();
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [load]);

  async function cancel() {
    setBusy(true);
    await fetch(`${base}/cancel?t=${encodeURIComponent(token)}`, { method: "POST" });
    setBusy(false);
    load();
  }

  const picker = <LangPicker lang={lang} setLang={setLang} label={e("language")} />;
  const locale = LOCALE[lang] ?? "en-IN";
  if (missing) return <div className="card state" role="alert"><h1>{e("orderMissingTitle")}</h1><p>{e("orderMissingBody")}</p>{picker}</div>;
  if (!view) return <div className="card state" role="status"><div className="spin" /><p>{e("orderLoading")}</p></div>;

  const [title, bodyKey] = TITLE[view.status];
  const ok = view.status === "paid";
  const pay = (() => {
    try {
      const s = sessionStorage.getItem(`zyvor:evt-pay:${view.order_id}`);
      return s && view.intent_id ? `/pay/${tenant}/${view.intent_id}?s=${encodeURIComponent(s)}&eo=${view.order_id}&et=${encodeURIComponent(token)}` : null;
    } catch {
      return null;
    }
  })();
  return (
    <div className="card evt">
      <p className="merchant">{view.merchant ?? e("eventTitle")}</p>
      <div className="state" role="status" aria-live="polite">
        {ok && <svg className="badge" viewBox="0 0 84 84" aria-hidden="true" stroke="var(--ok)"><circle cx="42" cy="42" r="40" /><path d="M26 43l11 11 21-23" /></svg>}
        {view.status === "pending" && <div className="spin" />}
        <h1 className="evt-title" tabIndex={-1} data-announce>{e(title)}</h1>
        <p>{e(bodyKey, { when: view.hold_expires_at ? when(view.hold_expires_at, locale) : "" })}</p>
      </div>
      <h2 className="evt-h">{view.event.name}</h2>
      <p className="desc" style={{ marginBottom: 4 }}>{when(view.event.starts_at, locale)}</p>
      <p className="desc" style={{ marginBottom: 14 }}>{view.event.mode === "online" ? e("online") : view.event.venue}</p>
      {ok && view.event.online_url && <a className="pay" href={view.event.online_url} rel="noopener noreferrer">{e("joinOnline")}</a>}
      {view.status === "pending" && pay && <a className="pay" href={pay}>{e("payNow")}</a>}
      {view.tickets.length > 0 && (
        <ul className="tickets">
          {view.tickets.map((k, i) => (
            <li key={k.id} className={k.status === "issued" ? "ticket" : "ticket off"}>
              <p className="tier-d" style={{ margin: 0 }}>{e("ticketOf", { n: String(i + 1), total: String(view.tickets.length) })} · {k.tier}</p>
              {k.qr_payload && (
                // eslint-disable-next-line @next/next/no-img-element
                <img className="qr" width={240} height={240} src={`/api/zyvor-events/${tenant}/orders/${order}/tickets/${k.id}/qr.svg?t=${encodeURIComponent(token)}`} alt={e("qrAlt", { code: k.code })} />
              )}
              <p className="code">{e("ticketCode", { code: k.code.replace(/(.{4})/g, "$1 ").trim() })}</p>
              <span className={k.status === "issued" ? "pill ok" : "pill"}>{e(TICKET[k.status] ?? "statusVoid")}</span>
            </li>
          ))}
        </ul>
      )}
      <dl className="rows" style={{ marginTop: 16 }}>
        <dt>{e("total")}</dt><dd>{view.total_minor === 0 ? e("free") : money(view.total_minor, view.currency)}</dd>
        <dt>{e("orderRef", { id: "" }).trim()}</dt><dd>{view.order_id.slice(-8).toUpperCase()}</dd>
      </dl>
      {view.status === "pending" && <button className="link" onClick={cancel} disabled={busy}>{busy ? e("cancelling") : e("cancelOrder")}</button>}
      {(view.status === "expired" || view.status === "cancelled") && <a className="link" href={`/events/${tenant}/${view.event.id}`}>{e("backToEvent")}</a>}
      {ok && <p className="foot">{e("keepPrivate")}</p>}
      <p className="foot">{t("secured")}</p>
      {picker}
    </div>
  );
}
