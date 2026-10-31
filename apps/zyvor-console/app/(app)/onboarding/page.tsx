import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { BankForm, DocForm, ProfileForm, ReviewForm } from "@/components/OnboardingForms";
import { submitOnboarding, verifyBankAccount } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { api } from "@/lib/zyvor";

export default async function Onboarding() {
  const [o, me] = await Promise.all([(await api()).onboarding.get(), currentSession()]);
  const admin = me?.r === "admin";
  const open = o.status === "not_started" || o.status === "draft" || o.status === "rejected";
  return (
    <>
      <div className="head"><div><h1>Onboarding</h1><p className="lede">Who the business is and where its money goes. A second person reviews this before payouts open. Zyvor keeps references and the last four digits only; it does not verify identity itself.</p></div><Chip status={o.status.replace(/_/g, " ")} /></div>
      {o.profile && <div className="card" style={{ marginBottom: 16 }}>
        <strong>{o.profile.legal_name}</strong> · {o.profile.business_type.replace(/_/g, " ")} · PAN {o.profile.pan}{o.profile.gstin ? ` · GSTIN ${o.profile.gstin}` : ""}
        <div className="muted">{o.profile.address}</div>
        {o.profile.review_note && <p className="note">Reviewer note: {o.profile.review_note}</p>}
      </div>}
      {open && admin && <ProfileForm />}
      <h2>Documents</h2>
      <div className="tablewrap"><table><thead><tr><th>Kind</th><th>Reference</th><th>Status</th></tr></thead><tbody>
        {o.documents.length === 0 && <tr><td colSpan={3} className="empty">None yet.</td></tr>}
        {o.documents.map((d) => <tr key={d.id}><td>{d.kind.replace(/_/g, " ")}</td><td className="mono">{d.reference}</td><td><Chip status={d.status} /></td></tr>)}
      </tbody></table></div>
      {open && admin && <DocForm />}
      <h2>Bank accounts</h2>
      <div className="tablewrap"><table><thead><tr><th>Holder</th><th>IFSC</th><th>Account</th><th>Verified</th><th /></tr></thead><tbody>
        {o.bank_accounts.length === 0 && <tr><td colSpan={5} className="empty">None yet.</td></tr>}
        {o.bank_accounts.map((b) => <tr key={b.id}><td>{b.holder_name}</td><td className="mono">{b.ifsc}</td><td className="mono">••••{b.account_last4}</td><td>{b.verified ? "Yes" : "No"}</td>
          <td>{!b.verified && admin && <Act action={verifyBankAccount} id={b.id} label="Verify" />}</td></tr>)}
      </tbody></table></div>
      {open && admin && <BankForm />}
      {open && admin && o.profile && <p><Act action={submitOnboarding} label="Submit for review" primary /></p>}
      {o.status === "in_review" && admin && <><h2>Review</h2><p className="muted">Submitted by {o.profile?.submitted_by}. You cannot review your own submission; verify a bank account before approving.</p><ReviewForm /></>}
    </>
  );
}
