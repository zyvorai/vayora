import Link from "next/link";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

const greeting = () => {
  const h = Number(new Intl.DateTimeFormat("en-IN", { hour: "numeric", hour12: false, timeZone: "Asia/Kolkata" }).format(new Date()));
  return h < 12 ? "Good morning" : h < 17 ? "Good afternoon" : "Good evening";
};

export default async function Overview() {
  const z = api();
  const [intents, refunds, cases, ageing, promises, disputes, cash, anomalies] = await Promise.all([
    z.intents.list({ limit: 50 }),
    z.refunds.list({ status: "pending_approval", limit: 50 }),
    z.cases.list({ status: "open" }),
    z.collections.ageing(),
    z.collections.promises("broken"),
    z.disputes.list("open"),
    z.insights.cashflow(4).catch(() => null),
    z.insights.refundAnomalies().catch(() => null),
  ]);
  const succeeded = intents.data.filter((i) => i.status === "succeeded");
  const inrBuckets = ageing.data.filter((b) => b.currency === "INR");
  const outstanding = inrBuckets.reduce((n, b) => n + b.outstanding_minor, 0);
  const overdue = inrBuckets.filter((b) => b.bucket !== "current" && b.bucket !== "no_due_date").reduce((n, b) => n + b.outstanding_minor, 0);

  const stats = [
    { k: "Collected", v: inr(succeeded.reduce((n, i) => n + i.amount_minor, 0)), n: `${succeeded.length} successful of your last ${intents.data.length} payments`, href: "/payments" },
    { k: "Outstanding", v: inr(outstanding), n: overdue ? `${inr(overdue)} is overdue` : "Nothing is overdue", href: "/collections", hot: overdue > 0 },
  ];
  const todo = [
    { n: refunds.data.length, text: "refunds waiting for a second person to approve", href: "/refunds", tone: "warn" },
    { n: cases.data.length, text: "reconciliation differences nobody has explained", href: "/reconciliation", tone: "warn" },
    { n: disputes.data.length, text: "open disputes", href: "/disputes", tone: "bad" },
    { n: promises.data.length, text: "broken promises to pay", href: "/collections", tone: "bad" },
    ...(anomalies?.flags ?? []).map((f) => ({ n: 1, text: f.message, href: "/refunds", tone: "warn" })),
  ].filter((t) => t.n > 0);

  return (
    <>
      <div className="head"><div><p className="hello">{greeting()}</p><h1>Here’s where things stand.</h1></div></div>
      <div className="grid stats">
        {stats.map((s) => (
          <Link key={s.k} href={s.href} className={`card stat${s.hot ? " hot" : ""}`} style={{ color: "inherit" }}>
            <div className="k">{s.k}</div><div className="v">{s.v}</div><div className="n">{s.n}</div>
          </Link>
        ))}
      </div>
      {cash && (
        <>
          <h2 style={{ marginTop: 28 }}>Due to come in</h2>
          <div className="grid stats">
            {cash.weeks.map((w, i) => (
              <div key={w.starts} className="card stat"><div className="k">{i === 0 ? "Next 7 days" : `Week ${i + 1}`}</div><div className="v">{inr(w.due_minor)}</div></div>
            ))}
            <div className="card stat"><div className="k">Already overdue</div><div className="v">{inr(cash.overdue_minor)}</div><div className="n">{cash.promised_minor ? `${inr(cash.promised_minor)} promised` : "None promised yet"}</div></div>
          </div>
          <p className="muted" style={{ marginTop: 8 }}>By due date, not a forecast of when people will pay.{cash.payouts_waiting_minor ? ` ${inr(cash.payouts_waiting_minor)} of payouts are waiting.` : ""}</p>
        </>
      )}
      <h2 style={{ marginTop: 28 }}>Needs a person</h2>
      <div className="card attn" style={{ padding: 0 }}>
        {todo.length === 0 && <div className="empty">All clear. Nothing needs you right now.</div>}
        {todo.map((t) => (
          <Link key={t.text} href={t.href}>
            <span className={`dot ${t.tone}`} /><span><b>{t.n}</b> <span className="m">{t.text}</span></span><span className="go">→</span>
          </Link>
        ))}
      </div>
    </>
  );
}
