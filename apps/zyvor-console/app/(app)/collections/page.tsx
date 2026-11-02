import Link from "next/link";
import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { DraftReview } from "@/components/DraftReview";
import { draftReminder, remind } from "@/lib/actions";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

const LABEL: Record<string, string> = { current: "Not yet due", "1_30": "Overdue 1–30 days", "31_60": "Overdue 31–60 days", "61_90": "Overdue 61–90 days", over_90: "Overdue over 90 days", no_due_date: "No due date" };

export default async function Collections() {
  const z = await api();
  const [ageing, promises, drafts, fx, customers] = await Promise.all([z.collections.ageing(), z.collections.promises(), z.collections.drafts(), z.fx.exposure(), z.customers.list()]);
  const who = new Map(customers.data.map((c) => [c.id, c.name]));
  const ai = process.env.CONSOLE_AI === "1";
  const rows = ageing.data.flatMap((b) => b.invoices.map((i) => ({ ...i, bucket: b.bucket, currency: b.currency })));
  return (
    <>
      <div className="head"><div><h1>Collections</h1><p className="lede">Who owes what, how late, and what they have promised. As of {ageing.as_of}.</p></div></div>
      {ageing.total_outstanding_inr_minor != null && ageing.data.some((b) => b.currency !== "INR") && <p className="muted">All currencies together: <strong>{inr(ageing.total_outstanding_inr_minor)}</strong> at your recorded dollar rate.</p>}
      <div className="grid stats">
        {ageing.data.map((b) => (
          <div key={b.currency + b.bucket} className="card stat">
            <div className="k">{LABEL[b.bucket] ?? b.bucket}{b.currency !== "INR" ? ` · ${b.currency}` : ""}</div>
            <div className="v">{inr(b.outstanding_minor)}</div><div className="n">{b.count} invoice{b.count === 1 ? "" : "s"}</div>
          </div>
        ))}
        {ageing.data.length === 0 && <div className="card stat"><div className="k">Outstanding</div><div className="v">₹0.00</div><div className="n">Nothing is owed.</div></div>}
      </div>
      {drafts.data.length > 0 && (
        <>
          <h2>Reminders to review</h2>
          <div style={{ display: "grid", gap: 12, marginBottom: 28 }}>
            {drafts.data.map((d) => <DraftReview key={d.id} id={d.id} invoice={d.invoice_number} channel={d.channel} message={d.message} source={d.source} />)}
          </div>
        </>
      )}
      {fx.data.length > 0 && (
        <>
          <h2>Dollar invoices</h2>
          <p className="muted">
            Value in rupees when each was raised and at {fx.rate_date ?? "no recorded rate"}.{" "}
            {fx.complete && fx.unrealised_inr_minor != null ? <>Difference <strong>{inr(fx.unrealised_inr_minor)}</strong>. </> : "Some days have no recorded rate, so the total is not shown. "}
            This is a report only; nothing is posted to your books.
          </p>
          <div className="tablewrap" style={{ marginBottom: 28 }}>
            <table>
              <thead><tr><th>Invoice</th><th>Raised</th><th className="num">Owed (USD)</th><th className="num">When raised</th><th className="num">Now</th><th className="num">Difference</th></tr></thead>
              <tbody>
                {fx.data.map((i) => (
                  <tr key={i.invoice_id}>
                    <td>{i.number}</td><td>{i.raised_on}</td><td className="num">${(i.outstanding_usd_minor / 100).toFixed(2)}</td>
                    <td className="num">{i.booked_inr_minor != null ? inr(i.booked_inr_minor) : "no rate"}</td>
                    <td className="num">{i.current_inr_minor != null ? inr(i.current_inr_minor) : "no rate"}</td>
                    <td className="num">{i.unrealised_inr_minor != null ? inr(i.unrealised_inr_minor) : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
      <h2>Unpaid invoices</h2>
      <div className="tablewrap" style={{ marginBottom: 28 }}>
        <table>
          <thead><tr><th>Invoice</th><th>Customer</th><th>Due</th><th className="num">Days late</th><th className="num">Outstanding</th><th /></tr></thead>
          <tbody>
            {rows.length === 0 && <tr><td colSpan={6} className="empty">No unpaid invoices. Overdue invoices from <Link href="/billing">Billing</Link> appear here, with their reminders.</td></tr>}
            {rows.map((i) => (
              <tr key={i.id}>
                <td>{i.number}</td><td>{(i.customer_id && who.get(i.customer_id)) || <span className="muted">—</span>}</td><td>{i.due_date ?? "—"}</td><td className="num">{i.days_past_due > 0 ? i.days_past_due : <span className="muted">—</span>}</td>
                <td className="num">{inr(i.outstanding_minor)}</td>
                <td style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
                  <Act action={draftReminder} id={i.id} label="Draft reminder" />
                  {ai && <Act action={draftReminder} id={i.id} label="Draft with AI" extra={{ ai: "1" }} />}
                  <Act action={remind} id={i.id} label="Send now" extra={{ channel: "whatsapp" }} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="muted" style={{ marginTop: -16, marginBottom: 28 }}>Send now records the reminder and announces it to your own messaging system (the <code>invoice.reminder</code> event). Zyvor emails the customer itself only if you turn on reminder emails in Settings and the reminder channel is email.</p>
      <h2>Promises to pay</h2>
      <div className="tablewrap">
        <table>
          <thead><tr><th>Invoice</th><th className="num">Promised</th><th>By</th><th>Status</th><th>Note</th></tr></thead>
          <tbody>
            {promises.data.length === 0 && <tr><td colSpan={5} className="empty">No promises to pay recorded. Note one on an unpaid invoice when a customer agrees to pay by a date.</td></tr>}
            {promises.data.map((p) => {
              const r = p as Record<string, string | number | null>;
              return (
                <tr key={String(r.id)}><td>{String(r.invoice_number)}</td><td className="num">{inr(Number(r.promised_minor))}</td><td>{String(r.promised_date)}</td><td><Chip status={String(r.status)} /></td><td className="muted">{r.note ?? ""}</td></tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}
