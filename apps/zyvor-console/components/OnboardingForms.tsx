"use client";
import { startTransition, useActionState, type FormEvent, type ReactNode } from "react";
import { addBankAccount, addOnboardingDoc, requestProfileChange, reviewOnboarding, saveProfile, type FormState } from "@/lib/actions";

function Card({ action, children, cta }: { action: (s: FormState, f: FormData) => Promise<FormState>; children: ReactNode; cta: string }) {
  const [state, run, pending] = useActionState(action, undefined);
  // Submitted by hand so a refusal keeps what was typed (a form `action` resets its fields afterwards).
  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    startTransition(() => run(data));
  };
  return (
    <form onSubmit={submit} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 16 }}>
      {children}
      <button className="btn primary" disabled={pending}>{pending ? "Saving…" : cta}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%" }}>{state.error}</p>}
      {state?.ok && <p className="note" role="status" style={{ flexBasis: "100%" }}>Saved.</p>}
    </form>
  );
}

const TYPES: [string, string][] = [["proprietorship", "Proprietorship"], ["partnership", "Partnership"], ["llp", "LLP"], ["private_limited", "Private limited"], ["trust_or_society", "Trust or society"], ["other", "Other"]];
const DOCS: [string, string][] = [["pan", "PAN card"], ["gst_certificate", "GST certificate"], ["incorporation", "Incorporation / registration"], ["address_proof", "Address proof"], ["cancelled_cheque", "Cancelled cheque"], ["other", "Other"]];

export function ProfileForm() {
  return (
    <Card action={saveProfile} cta="Save profile">
      <label className="field" style={{ flex: "1 1 200px" }}>Legal name<input name="legal_name" required /></label>
      <label className="field" style={{ flex: "0 1 180px" }}>Business type<select name="business_type" defaultValue="proprietorship">{TYPES.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select></label>
      <label className="field" style={{ flex: "0 1 140px" }}>PAN<input name="pan" maxLength={10} autoCapitalize="characters" placeholder="ABCDE1234F" required /></label>
      <label className="field" style={{ flex: "0 1 190px" }}>GSTIN (optional)<input name="gstin" maxLength={15} autoCapitalize="characters" /></label>
      <label className="field" style={{ flex: "1 1 100%" }}>Registered address<input name="address" required /></label>
    </Card>
  );
}

/** After approval the details are locked; this asks Zyvor to change them. The PAN is asked again because it is checked and only its last four digits are kept. */
export function ProfileChangeForm({ current }: { current: { legal_name: string; business_type: string; gstin: string | null; address: string } }) {
  return (
    <Card action={requestProfileChange} cta="Ask Zyvor to change these">
      <label className="field" style={{ flex: "1 1 200px" }}>Legal name<input name="legal_name" defaultValue={current.legal_name} required /></label>
      <label className="field" style={{ flex: "0 1 180px" }}>Business type<select name="business_type" defaultValue={current.business_type}>{TYPES.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select></label>
      <label className="field" style={{ flex: "0 1 140px" }}>PAN<input name="pan" maxLength={10} autoCapitalize="characters" placeholder="ABCDE1234F" required /></label>
      <label className="field" style={{ flex: "0 1 190px" }}>GSTIN (optional)<input name="gstin" maxLength={15} autoCapitalize="characters" defaultValue={current.gstin ?? ""} /></label>
      <label className="field" style={{ flex: "1 1 100%" }}>Registered address<input name="address" defaultValue={current.address} required /></label>
      <label className="field" style={{ flex: "1 1 100%" }}>Why are they changing?<input name="reason" required minLength={5} placeholder="e.g. converted to a private limited company" /></label>
    </Card>
  );
}

export function DocForm() {
  return (
    <Card action={addOnboardingDoc} cta="Add document">
      <label className="field" style={{ flex: "0 1 220px" }}>Kind<select name="kind" defaultValue="pan">{DOCS.map(([v, l]) => <option key={v} value={v}>{l}</option>)}</select></label>
      <label className="field" style={{ flex: "1 1 280px" }}>Where the file is kept (link or path)<input name="reference" required /></label>
    </Card>
  );
}

export function BankForm() {
  return (
    <Card action={addBankAccount} cta="Add bank account">
      <label className="field" style={{ flex: "1 1 200px" }}>Account holder<input name="holder_name" required /></label>
      <label className="field" style={{ flex: "0 1 150px" }}>IFSC<input name="ifsc" maxLength={11} autoCapitalize="characters" placeholder="HDFC0001234" required /></label>
      <label className="field" style={{ flex: "0 1 200px" }}>Account number<input name="account_number" inputMode="numeric" autoComplete="off" required /></label>
    </Card>
  );
}

export function ReviewForm() {
  const [state, run, pending] = useActionState(reviewOnboarding, undefined);
  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    const who = (e.nativeEvent as SubmitEvent).submitter as HTMLButtonElement | null;
    if (who?.name) data.set(who.name, who.value);
    startTransition(() => run(data));
  };
  return (
    <form onSubmit={submit} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end" }}>
      <label className="field" style={{ flex: "1 1 280px" }}>Note (required to reject)<input name="note" /></label>
      <button className="btn primary" name="decision" value="approve" disabled={pending}>Approve</button>
      <button className="btn" name="decision" value="reject" disabled={pending}>Reject</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%" }}>{state.error}</p>}
    </form>
  );
}
