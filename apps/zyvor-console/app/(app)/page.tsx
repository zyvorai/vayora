import Link from "next/link";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function Overview() {
  const z = api();
  const [intents, refunds, cases, ageing, promises] = await Promise.all([
    z.intents.list({ limit: 50 }),
    z.refunds.list({ status: "pending_approval", limit: 50 }),
    z.cases.list({ status: "open" }),
    z.collections.ageing(),
    z.collections.promises("broken"),
  ]);
  const succeeded = intents.data.filter((i) => i.status === "succeeded");
  const overdue = ageing.data.filter((b) => b.bucket !== "current" && b.bucket !== "no_due_date" && b.currency === "INR").reduce((n, b) => n + b.outstanding_minor, 0);
  const outstanding = ageing.data.filter((b) => b.currency === "INR").reduce((n, b) => n + b.outstanding_minor, 0);
  const stats = [
    { k: "Collected (last 50 payments)", v: inr(succeeded.reduce((n, i) => n + i.amount_minor, 0)), n: `${succeeded.length} succeeded of ${intents.data.length}`, href: "/payments" },
    { k: "Outstanding", v: inr(outstanding), n: `${inr(overdue)} overdue`, href: "/collections" },
    { k: "Refunds awaiting approval", v: String(refunds.data.length), n: "Needs a second person", href: "/refunds" },
    { k: "Open reconciliation cases", v: String(cases.data.length), n: "Differences nobody has explained yet", href: "/reconciliation" },
    { k: "Broken promises to pay", v: String(promises.data.length), n: "Date passed, still unpaid", href: "/collections" },
  ];
  return (
    <>
      <div className="head"><div><h1>Overview</h1><p className="lede">What needs a person today.</p></div></div>
      <div className="grid stats">
        {stats.map((s) => (
          <Link key={s.k} href={s.href} className="card stat" style={{ color: "inherit", textDecoration: "none" }}>
            <div className="k">{s.k}</div><div className="v">{s.v}</div><div className="n">{s.n}</div>
          </Link>
        ))}
      </div>
    </>
  );
}
