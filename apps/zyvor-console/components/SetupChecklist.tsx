import Link from "next/link";
import { api } from "@/lib/zyvor";

/** What an admin still has to do to be ready for real payments. Hidden once everything is done, or if any lookup fails. */
export async function SetupChecklist() {
  const z = await api();
  let steps: { label: string; hint: string; href: string; done: boolean }[];
  try {
    const [o, keys, s, links, access, pays] = await Promise.all([z.onboarding.get(), z.keys.list(), z.settings.get(), z.paymentLinks.list(), z.liveAccess.get(), z.intents.list({ limit: 1 })]);
    // quick wins first: see a payment work, then the things that take time (a review) come last
    steps = [
      ...(s.env === "test" ? [{ label: "Take a test payment", hint: "See a payment go through, with no real money", href: "/sandbox", done: pays.data.length > 0 }] : []),
      { label: "Create a payment link", hint: "The quickest way to get paid", href: "/links", done: links.data.length > 0 },
      { label: "Create an API key", hint: "To take payments from your own app", href: "/keys", done: keys.data.some((k) => !k.revoked_at && !k.name.startsWith("console session")) },
      { label: "Add a webhook address", hint: "To hear about payments as they happen", href: "/settings", done: !!s.outbound_webhook_url },
      { label: "Get your business details approved", hint: o.status === "in_review" ? "Waiting for review" : "PAN, bank account and documents", href: "/onboarding", done: o.status === "approved" },
      { label: "Get live access", hint: access.status === "pending" ? "Zyvor is reviewing your business" : access.status === "rejected" ? "Zyvor asked for a change" : "Zyvor reviews every business before live mode opens", href: "/onboarding", done: access.status === "approved" },
    ];
  } catch {
    return null;
  }
  const done = steps.filter((x) => x.done).length;
  const next = steps.find((x) => !x.done);
  if (done === steps.length) return null;
  return (
    <section className="card" style={{ marginBottom: 14 }} aria-label="Finish setting up">
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", gap: 12, flexWrap: "wrap" }}>
        <h2 style={{ margin: 0 }}>Finish setting up</h2><span className="muted">{done} of {steps.length} done</span>
      </div>
      <div className="meter" role="progressbar" aria-valuemin={0} aria-valuemax={steps.length} aria-valuenow={done} aria-label="Setup progress"><span style={{ width: `${(done / steps.length) * 100}%` }} /></div>
      <ul className="checklist">
        {steps.map((x) => (
          <li key={x.label} className={x.done ? "done" : ""}>
            <span className="tick" aria-hidden="true">{x.done ? "✓" : ""}</span>
            <Link href={x.href}><strong>{x.label}</strong><span className="muted"> · {x.hint}</span><span className="sr">{x.done ? " (done)" : ""}</span></Link>
            {x === next && <span className="chip accent" style={{ marginLeft: "auto" }}>Next</span>}
          </li>
        ))}
      </ul>
    </section>
  );
}
