"use client";
import { startTransition, useActionState, type FormEvent, type ReactNode } from "react";
import { addCash, createBranch, createClosing, createCounter, type FormState } from "@/lib/actions";

function Card({ action, children, cta }: { action: (s: FormState, f: FormData) => Promise<FormState>; children: ReactNode; cta: string }) {
  const [state, run, pending] = useActionState(action, undefined);
  // Submitted by hand so a refusal keeps what was typed.
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

type Branch = { id: string; name: string };
const BranchSelect = ({ branches }: { branches: Branch[] }) => (
  <label className="field" style={{ flex: "0 1 200px" }}>Branch<select name="branch_id" required>{branches.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}</select></label>
);

export function BranchForm() {
  return <Card action={createBranch} cta="Add branch"><label className="field" style={{ flex: "1 1 220px" }}>Branch name<input name="name" required /></label></Card>;
}
export function CounterForm({ branches }: { branches: Branch[] }) {
  return <Card action={createCounter} cta="Add counter"><BranchSelect branches={branches} /><label className="field" style={{ flex: "1 1 200px" }}>Counter name (till, desk, stall)<input name="name" required /></label></Card>;
}
export function CashForm({ branches }: { branches: Branch[] }) {
  return (
    <Card action={addCash} cta="Record cash">
      <BranchSelect branches={branches} />
      <label className="field" style={{ flex: "0 1 140px" }}>What<select name="kind" defaultValue="sale"><option value="sale">Cash sale</option><option value="refund">Cash refund</option><option value="float">Opening float</option></select></label>
      <label className="field" style={{ flex: "0 1 140px" }}>Amount (₹)<input name="amount" inputMode="decimal" required /></label>
      <label className="field" style={{ flex: "1 1 160px" }}>Note<input name="note" /></label>
    </Card>
  );
}
export function ClosingForm({ branches, today }: { branches: Branch[]; today: string }) {
  return (
    <Card action={createClosing} cta="Close the day">
      <BranchSelect branches={branches} />
      <label className="field" style={{ flex: "0 1 160px" }}>Day<input name="day" type="date" defaultValue={today} max={today} required /></label>
      <label className="field" style={{ flex: "0 1 160px" }}>Cash counted (₹)<input name="counted" inputMode="decimal" required /></label>
      <label className="field" style={{ flex: "1 1 160px" }}>Note<input name="note" /></label>
    </Card>
  );
}
