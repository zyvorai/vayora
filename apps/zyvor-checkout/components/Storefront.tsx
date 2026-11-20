// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

"use client";
import { useEffect, useMemo, useState } from "react";
import { LangPicker, useI18n } from "@/components/useI18n";
import { money } from "@/lib/checkout";
import { cartLines, cartTotal, clampCart, maxFor, orderErrorKey, phoneOk, type Shop } from "@/lib/storefront";
import type { Key } from "@/lib/i18n";

/** A seller's page: items to choose, a short form, then the ordinary checkout. Everything from the server is shown as text. */
export function Storefront({ slug }: { slug: string }) {
  const { lang, setLang, t } = useI18n();
  const base = `/api/store/${encodeURIComponent(slug)}`;
  const [shop, setShop] = useState<Shop | null>(null);
  const [missing, setMissing] = useState(false);
  const [qty, setQty] = useState<Record<string, number>>({});
  const [name, setName] = useState("");
  const [phone, setPhone] = useState("");
  const [email, setEmail] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<Key | null>(null);
  const [touched, setTouched] = useState(false);

  useEffect(() => {
    fetch(base)
      .then((r) => (r.ok ? r.json() : Promise.reject()))
      .then((s: Shop) => setShop(s))
      .catch(() => setMissing(true));
  }, [base]);

  const items = shop?.items ?? [];
  const chosen = useMemo(() => clampCart(qty, items), [qty, items]);
  const total = cartTotal(chosen, items);
  const lines = cartLines(chosen);
  const nameOk = name.trim().length > 0;
  const phoneGood = phoneOk(phone);

  function bump(id: string, by: number, max: number) {
    setError(null);
    setQty((q) => ({ ...q, [id]: Math.min(max, Math.max(0, (q[id] ?? 0) + by)) }));
  }

  async function order() {
    setTouched(true);
    if (!shop || !lines.length || !nameOk || !phoneGood) return;
    setBusy(true);
    setError(null);
    try {
      const r = await fetch(`${base}/orders`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ buyer_name: name, buyer_phone: phone, buyer_email: email || undefined, delivery_note: note || undefined, items: lines }),
      });
      const j = await r.json().catch(() => ({}));
      if (!r.ok) {
        setError(orderErrorKey(j?.error?.code, r.status));
        // stock may have changed: show the page as it is now
        if (j?.error?.code === "out_of_stock" || j?.error?.code === "item_unavailable") fetch(base).then((x) => (x.ok ? x.json() : null)).then((s) => s && setShop(s));
        setBusy(false);
        return;
      }
      window.location.assign(`/pay/${j.tenant_id}/${j.intent_id}?s=${encodeURIComponent(j.client_secret)}`);
    } catch {
      setError("storeErrGeneric");
      setBusy(false);
    }
  }

  const picker = <LangPicker lang={lang} setLang={setLang} label={t("language")} />;
  if (missing) return <div className="card state" role="alert"><h1>{t("storeGoneTitle")}</h1><p>{t("storeGoneBody")}</p>{picker}</div>;
  if (!shop) return <div className="card state" role="status"><div className="spin" /><p>{t("loadingShort")}</p></div>;

  return (
    <div className="card shop">
      <header className="shophead">
        {shop.logo_url && <img className="shoplogo" src={shop.logo_url} alt="" width={64} height={64} />}
        <h1>{shop.title}</h1>
        {shop.description && <p className="desc shopdesc">{shop.description}</p>}
        {shop.merchant && <p className="soldby">{t("storeSoldBy", { merchant: shop.merchant })}</p>}
      </header>
      {items.length === 0 ? <p className="desc">{t("storeEmpty")}</p> : (
        <ul className="items">
          {items.map((it) => {
            const max = maxFor(it);
            const q = chosen[it.id] ?? 0;
            return (
              <li key={it.id} className="item">
                {it.image_url && <img className="itemimg" src={it.image_url} alt="" width={72} height={72} loading="lazy" decoding="async" />}
                <div className="iteminfo">
                  <p className="itemname">{it.name}</p>
                  {it.description && <p className="itemdesc">{it.description}</p>}
                  <p className="itemprice">{money(it.price_minor, shop.currency)}{it.available !== null && it.available > 0 && it.available <= 5 && <span className="low"> · {t("storeLeft", { n: String(it.available) })}</span>}</p>
                </div>
                {it.sold_out ? <span className="soldout">{t("storeSoldOut")}</span> : q === 0 ? (
                  <button type="button" className="add" onClick={() => bump(it.id, 1, max)}>{t("storeAdd")}</button>
                ) : (
                  <div className="stepper" role="group" aria-label={it.name}>
                    <button type="button" aria-label={t("storeLess", { item: it.name })} onClick={() => bump(it.id, -1, max)}>−</button>
                    <span aria-live="polite">{q}</span>
                    <button type="button" aria-label={t("storeMore", { item: it.name })} onClick={() => bump(it.id, 1, max)} disabled={q >= max}>+</button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {lines.length > 0 && (
        <form onSubmit={(e) => { e.preventDefault(); void order(); }} noValidate>
          <h2 className="yourorder">{t("storeYourOrder")}</h2>
          <dl className="rows"><dt>{t("storeTotal")}</dt><dd>{money(total, shop.currency)}</dd></dl>
          <label className="f" htmlFor="sn">{t("storeName")}</label>
          <input id="sn" type="text" value={name} onChange={(e) => setName(e.target.value)} maxLength={80} autoComplete="name" aria-invalid={touched && !nameOk} />
          {touched && !nameOk && <p className="err" role="status">{t("storeErrName")}</p>}
          <label className="f gap" htmlFor="sp">{t("storePhone")}</label>
          <input id="sp" type="text" inputMode="tel" value={phone} onChange={(e) => setPhone(e.target.value)} maxLength={20} autoComplete="tel" aria-invalid={touched && !phoneGood} />
          {touched && !phoneGood && <p className="err" role="status">{t("storeErrPhone")}</p>}
          <label className="f gap" htmlFor="se">{t("storeEmail")}</label>
          <input id="se" type="text" inputMode="email" value={email} onChange={(e) => setEmail(e.target.value)} maxLength={120} autoComplete="email" />
          <label className="f gap" htmlFor="sd">{t("storeNote")}</label>
          <textarea id="sd" value={note} onChange={(e) => setNote(e.target.value)} maxLength={300} rows={3} aria-describedby="sdh" />
          <p className="foot hint" id="sdh">{t("storeNoteHint")}</p>
          {error && <div className="notice bad" role="alert">{t(error)}</div>}
          <button className="pay" type="submit" disabled={busy}>{busy ? t("storePlacing") : t("storeCheckout", { amount: money(total, shop.currency) })}</button>
          <p className="foot">{t("storeHold", { minutes: "30" })}</p>
        </form>
      )}
      <p className="foot">{t("secured")}</p>
      {picker}
    </div>
  );
}
