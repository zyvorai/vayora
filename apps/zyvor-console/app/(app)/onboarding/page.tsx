import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { BankForm, DocForm, ProfileForm, ReviewForm } from "@/components/OnboardingForms";
import { Stepper, type Step } from "@/components/Stepper";
import { submitOnboarding, verifyBankAccount } from "@/lib/actions";
import { currentSession, requireRole } from "@/lib/auth";
import { api } from "@/lib/zyvor";

const KIND: Record<string, string> = { pan: "PAN", gstin: "GSTIN", bank: "Bank account" };

export default async function Onboarding({ searchParams }: { searchParams: Promise<{ step?: string }> }) {
  await requireRole("finance");
  const [o, me, q] = await Promise.all([(await api()).onboarding.get(), currentSession(), searchParams]);
  const admin = me?.r === "admin";
  const open = o.status === "not_started" || o.status === "draft" || o.status === "rejected";
  const hasPan = o.documents.some((d) => d.kind === "pan");
  // once submitted, every earlier step counts as done (an approval carried into live has no documents of its own)
  const submitted = o.status === "in_review" || o.status === "approved";
  const steps: Step[] = [
    { key: "business", label: "Business", done: submitted || !!o.profile },
    { key: "documents", label: "Documents", done: submitted || hasPan },
    { key: "bank", label: "Bank account", done: submitted || o.bank_accounts.length > 0 },
    { key: "review", label: "Review and submit", done: submitted },
  ];
  const firstOpen = steps.find((s) => !s.done)?.key ?? "review";
  const current = steps.some((s) => s.key === q.step) ? q.step! : firstOpen;
  const next = steps[steps.findIndex((s) => s.key === current) + 1];
  const live = me?.e === "live";
  const checkFor = (kind: string, subject?: string) => o.checks.find((c) => c.kind === kind && (subject ? c.subject === subject : true));
  const Next = () => next && <p><a className="btn primary" href={`/onboarding?step=${next.key}`}>Continue to {next.label.toLowerCase()}</a></p>;

  return (
    <>
      <div className="head"><div><h1>Get set up</h1><p className="lede">Tell us who the business is and where its money goes. {o.automated_checks ? "Your PAN, GSTIN and bank account are checked as you add them, and a clean submission is approved straight away." : "A second person reviews the submission before payouts and live mode open."} We keep references and the last four digits only.</p></div><Chip status={o.status.replace(/_/g, " ")} /></div>

      {o.status === "approved" && (
        <div className="note ok" role="status" style={{ marginBottom: 16 }}>
          <strong>You are approved.</strong> {o.profile?.reviewed_by === "system:auto" ? "Every check passed, so no one needed to review it." : `Reviewed by ${o.profile?.reviewed_by}.`}{" "}
          {live ? "You are in live mode." : "Use the Test / Live switch at the top to go live."}
        </div>
      )}
      {o.status === "in_review" && (
        <div className="note info" role="status" style={{ marginBottom: 16 }}>
          <strong>Waiting for a person to review.</strong>{" "}
          {o.review_reasons.length > 0 ? <>It could not be approved automatically because:<ul style={{ margin: "6px 0 0", paddingLeft: 20 }}>{o.review_reasons.map((r) => <li key={r}>{r}</li>)}</ul></> : "Someone other than the person who submitted it needs to approve it."}
        </div>
      )}
      {o.status === "rejected" && o.profile?.review_note && <div className="note bad" role="alert" style={{ marginBottom: 16 }}><strong>Not approved.</strong> {o.profile.review_note} Fix the details below and submit again.</div>}

      <Stepper steps={steps} current={current} base="/onboarding" />

      {current === "business" && (
        <section>
          <h2>Business details</h2>
          {o.profile && <div className="card" style={{ marginBottom: 16 }}>
            <strong>{o.profile.legal_name}</strong> · {o.profile.business_type.replace(/_/g, " ")} · PAN {o.profile.pan}{o.profile.gstin ? ` · GSTIN ${o.profile.gstin}` : ""}
            <div className="muted">{o.profile.address}</div>
            {["pan", "gstin"].map((k) => { const c = checkFor(k); return c && <div key={k} className="muted" style={{ marginTop: 6 }}><Chip status={c.outcome} /> {KIND[k]}: {c.detail}</div>; })}
          </div>}
          {open && admin ? <ProfileForm /> : !o.profile && <p className="muted">An admin needs to enter the business details.</p>}
          {o.profile && <Next />}
        </section>
      )}

      {current === "documents" && (
        <section>
          <h2>Documents</h2>
          <p className="muted">A PAN document is required. Add where the file is kept; we do not store the file itself.</p>
          <div className="tablewrap"><table><thead><tr><th>Kind</th><th>Reference</th><th>Status</th></tr></thead><tbody>
            {o.documents.length === 0 && <tr><td colSpan={3} className="empty">None yet.</td></tr>}
            {o.documents.map((d) => <tr key={d.id}><td>{d.kind.replace(/_/g, " ")}</td><td className="mono">{d.reference}</td><td><Chip status={d.status} /></td></tr>)}
          </tbody></table></div>
          {open && admin && <div style={{ marginTop: 16 }}><DocForm /></div>}
          {hasPan && <Next />}
        </section>
      )}

      {current === "bank" && (
        <section>
          <h2>Bank account</h2>
          <p className="muted">{o.automated_checks ? "We confirm the account with a small test deposit when you add it." : "A second person confirms the account belongs to the business."}</p>
          <div className="tablewrap"><table><thead><tr><th>Holder</th><th>IFSC</th><th>Account</th><th>Verification</th><th /></tr></thead><tbody>
            {o.bank_accounts.length === 0 && <tr><td colSpan={5} className="empty">None yet.</td></tr>}
            {o.bank_accounts.map((b) => { const c = checkFor("bank", b.id); return <tr key={b.id}><td>{b.holder_name}</td><td className="mono">{b.ifsc}</td><td className="mono">••••{b.account_last4}</td>
              <td>{b.verified ? <Chip status="approved" /> : c ? <><Chip status={c.outcome} /> <span className="muted">{c.detail}</span></> : <span className="muted">Waiting for a person</span>}</td>
              <td>{!b.verified && admin && !o.automated_checks && <Act action={verifyBankAccount} id={b.id} label="Verify" />}</td></tr>; })}
          </tbody></table></div>
          {open && admin && <div style={{ marginTop: 16 }}><BankForm /></div>}
          {o.bank_accounts.length > 0 && <Next />}
        </section>
      )}

      {current === "review" && (
        <section>
          <h2>Review and submit</h2>
          <div className="card" style={{ display: "grid", gap: 8 }}>
            <div><strong>Business</strong> {o.profile ? <span className="muted">{o.profile.legal_name} · {o.profile.business_type.replace(/_/g, " ")}</span> : <span className="muted">not entered</span>}</div>
            <div><strong>Documents</strong> <span className="muted">{submitted && o.documents.length === 0 ? "reviewed in the test environment" : `${o.documents.length} added${hasPan ? "" : " (PAN missing)"}`}</span></div>
            <div><strong>Bank accounts</strong> <span className="muted">{submitted && o.bank_accounts.length === 0 ? "reviewed in the test environment" : `${o.bank_accounts.length} added, ${o.bank_accounts.filter((b) => b.verified).length} verified`}</span></div>
            {o.checks.length > 0 && <div><strong>Checks</strong><ul style={{ margin: "6px 0 0", paddingLeft: 20 }}>{o.checks.map((c) => <li key={c.kind + c.subject}><Chip status={c.outcome} /> {KIND[c.kind]}: {c.detail}</li>)}</ul></div>}
          </div>
          {open && admin && o.profile && hasPan && o.bank_accounts.length > 0
            ? <div style={{ marginTop: 16 }}><Act action={submitOnboarding} label={o.automated_checks ? "Submit and get approved" : "Submit for review"} primary /></div>
            : open && <p className="muted" style={{ marginTop: 16 }}>Finish the earlier steps to submit.</p>}
          {o.status === "in_review" && admin && !o.automated_checks && <><h2 style={{ marginTop: 24 }}>Decision</h2><p className="muted">Submitted by {o.profile?.submitted_by}. You cannot review your own submission; verify a bank account before approving.</p><ReviewForm /></>}
          {o.status === "in_review" && admin && o.automated_checks && <><h2 style={{ marginTop: 24 }}>Decision</h2><p className="muted">Submitted by {o.profile?.submitted_by}. Someone else must decide; you cannot review your own submission.</p><ReviewForm /></>}
        </section>
      )}
    </>
  );
}
