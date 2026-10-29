import { notFound } from "next/navigation";
import { randomUUID } from "node:crypto";
import { VayoraError } from "@vayora/node";
import { Chip } from "@/components/Chip";
import { RefundForm } from "@/components/RefundForm";
import { inr, istDateTime } from "@/lib/format";
import { api } from "@/lib/vayora";

export default async function PaymentDetail({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const v = api();
  const p = await v.payments.retrieve(id).catch((e) => {
    if (e instanceof VayoraError && e.status === 404) return null;
    throw e;
  });
  if (!p) notFound();
  const [order, refunds] = await Promise.all([
    v.orders.retrieve(p.order_id),
    // Refunds are listed newest-first across the account; keep this payment's.
    v.refunds.list({ limit: 100 }).then((r) => r.data.filter((x) => x.payment_id === p.id).reverse()),
  ]);
  const remaining = p.amount_paise - p.refunded_paise;

  return (
    <>
      <div className="head">
        <div>
          <p className="muted" style={{ margin: 0 }}><a href="/payments">Payments</a> /</p>
          <h1>{inr(p.amount_paise)} <Chip status={p.status} /></h1>
          <p className="lede mono">{p.id}</p>
        </div>
      </div>

      <div className="grid two">
        <div className="grid">
          <section className="card" aria-labelledby="details">
            <h2 id="details">Details</h2>
            <dl className="kv">
              <dt>Order</dt><dd className="mono">{order.id} · <Chip status={order.status} /></dd>
              <dt>UPI ID</dt><dd className="mono">{p.vpa}</dd>
              <dt>UPI reference</dt><dd className="mono">{p.upi_ref ?? "n/a"}</dd>
              {p.failure_reason && <><dt>Failure</dt><dd>{p.failure_reason.replace("_", " ")}</dd></>}
              <dt>Refunded</dt><dd>{inr(p.refunded_paise)} of {inr(p.amount_paise)}</dd>
              <dt>Created</dt><dd>{istDateTime(p.created_at)} IST</dd>
            </dl>
          </section>
          <section className="card" aria-labelledby="tl">
            <h2 id="tl">Timeline</h2>
            <ol className="timeline">
              <li><b>Order created</b><time>{istDateTime(order.created_at)}</time></li>
              <li className={p.status === "captured" ? "ok" : "bad"}><b>{p.status === "captured" ? "Payment captured" : "Payment failed"}</b><time>{istDateTime(p.created_at)}</time></li>
              {refunds.map((r) => (<li key={r.id}><b>Refunded {inr(r.amount_paise)}</b><time>{istDateTime(r.created_at)} · UPI ref {r.upi_ref}</time></li>))}
            </ol>
          </section>
        </div>
        <section className="card" aria-labelledby="refund">
          <h2 id="refund">Refund</h2>
          {p.status !== "captured" ? <p className="muted">Only captured payments can be refunded.</p>
            : remaining === 0 ? <p className="muted">Fully refunded.</p>
            : <RefundForm paymentId={p.id} idempotencyKey={randomUUID()} remaining={inr(remaining)} />}
        </section>
      </div>
    </>
  );
}
