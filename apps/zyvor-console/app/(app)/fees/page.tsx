// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { Chip } from "@/components/Chip";
import { ConfirmAct } from "@/components/ConfirmAct";
import { SettingsForm } from "@/components/SettingsForm";
import { currentSession } from "@/lib/auth";
import { FEE_METHODS, groupLabel, ruleSentence, varianceText, type FeeFinding, type FeeRule, type FeeSettings, type FeeSummary } from "@/lib/feecheck";
import { dayYear, inr } from "@/lib/format";
import { methodLabel, providerLabel } from "@/lib/labels";
import { addFeeRule, deleteFeeRule, runFeeCheck, saveFeeSettings } from "@/lib/recon-actions";
import { rest } from "@/lib/rest";
import { query } from "@/lib/restcall";
import { allows } from "@/lib/session";

const STATUSES = [["open", "Still to review"], ["reviewed", "Reviewed"], ["all", "All"]] as const;

export default async function Fees({ searchParams }: { searchParams: Promise<{ status?: string; provider?: string; method?: string }> }) {
  const sp = await searchParams;
  const status = STATUSES.some(([k]) => k === sp.status) ? (sp.status as string) : "open";
  const provider = (sp.provider ?? "").trim().slice(0, 40);
  const method = (FEE_METHODS as readonly string[]).includes(sp.method ?? "") ? sp.method : undefined;
  const me = await currentSession();
  const canWrite = !!me && allows(me.r, "finance");
  const r = await rest();
  const [summary, findings, rules, settings] = await Promise.all([
    r.get<FeeSummary>("/v1/fee_checks/summary"),
    r.get<{ data: FeeFinding[]; note: string }>(`/v1/fee_checks/findings${query({ status, provider: provider || undefined, method, limit: 100 })}`),
    r.get<{ data: FeeRule[] }>("/v1/fee_rules"),
    r.get<FeeSettings>("/v1/fee_checks/settings"),
  ]);
  const head = (
    <div className="head"><div><h1>Fee checks</h1><p className="lede">Compares the fee each provider took from a settlement with the rates in your own contract, payment by payment. A difference is a question for a person to look into, not proof of a mistake. Nothing here moves money.</p></div></div>
  );
  if (!summary.ok || !findings.ok || !rules.ok || !settings.ok) {
    const why = [summary, findings, rules, settings].find((x) => !x.ok);
    return <>{head}<p className="note bad" role="alert">{why && !why.ok ? why.message : "Fee checks could not be loaded."}</p></>;
  }
  const s = summary.body;
  const set = settings.body;
  const known = Array.from(new Set(rules.body.data.map((x) => x.provider)));
  return (
    <>
      {head}
      <p className="muted" style={{ marginTop: 0, maxWidth: "70ch" }}>Zyvor does not know any provider&apos;s rates. Every rate here is one you typed in from your own contract or the provider&apos;s fee schedule. Fee checks have been tested on simulated settlements only, not on a real provider statement.</p>
      {s.rules_configured === 0 && <p className="note warn" role="status">You have not entered any fee rules yet, so nothing is being checked. No finding does not mean the fees are right: there is just nothing to compare with. Add your first rule below.</p>}

      <div className="grid stats">
        <div className="card stat"><div className="k">Differences to review</div><div className="v">{s.open_findings}</div><div className="n">payments, across {s.by_provider_method.reduce((n, g) => n + g.settlements, 0)} settlement(s)</div></div>
        <div className="card stat"><div className="k">Provider took more than your rate</div><div className="v">{inr(s.over_deducted_minor)}</div><div className="n">worth asking the provider about</div></div>
        <div className="card stat"><div className="k">Provider took less than your rate</div><div className="v">{inr(s.under_deducted_minor)}</div><div className="n">also worth understanding</div></div>
        <div className="card stat"><div className="k">Payments with no rule</div><div className="v">{s.lines_without_a_rule}</div><div className="n">not checked at all</div></div>
      </div>
      {s.by_provider_method.length > 0 && (
        <div className="tablewrap" style={{ marginBottom: 28 }}>
          <table>
            <caption className="sr">Differences to review, by provider and method</caption>
            <thead><tr><th scope="col">Provider and method</th><th scope="col" className="num">Payments</th><th scope="col" className="num">Took more</th><th scope="col" className="num">Took less</th><th scope="col" className="num">Net</th></tr></thead>
            <tbody>
              {s.by_provider_method.map((g) => (
                <tr key={`${g.provider}/${g.method}`}>
                  <th scope="row" style={{ fontWeight: 500, color: "var(--fg)", textTransform: "none", fontSize: "inherit" }}>{groupLabel(g)}</th>
                  <td className="num">{g.open_findings}</td><td className="num">{inr(g.over_deducted_minor)}</td><td className="num">{inr(g.under_deducted_minor)}</td><td className="num">{varianceText(g.total_variance_minor)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <h2 id="findings" style={{ scrollMarginTop: 72 }}>Payments whose fee differs from your rate</h2>
      <form method="get" action="/fees" style={{ display: "flex", gap: 12, alignItems: "end", flexWrap: "wrap", marginBottom: 14 }}>
        <label className="field">Show<select name="status" defaultValue={status}>{STATUSES.map(([k, l]) => <option key={k} value={k}>{l}</option>)}</select></label>
        <label className="field">Provider<input name="provider" defaultValue={provider} list="fee-providers" autoComplete="off" /></label>
        <label className="field">Method<select name="method" defaultValue={method ?? ""}><option value="">All</option>{FEE_METHODS.map((m) => <option key={m} value={m}>{methodLabel(m)}</option>)}</select></label>
        <button className="btn small">Filter</button>
      </form>
      <datalist id="fee-providers">{known.map((p) => <option key={p} value={p} />)}</datalist>
      <div className="tablewrap" style={{ marginBottom: 8 }}>
        <table>
          <caption className="sr">Payments whose fee differs from your configured rate</caption>
          <thead><tr><th scope="col">Settled on</th><th scope="col">Provider and method</th><th scope="col" className="num">Payment</th><th scope="col" className="num">Fee taken</th><th scope="col" className="num">Your rule says</th><th scope="col">Difference</th><th scope="col">Review</th></tr></thead>
          <tbody>
            {findings.body.data.length === 0 && <tr><td colSpan={7} className="empty">{s.rules_configured === 0 ? "Nothing to show: no fee rules are entered." : status === "open" ? "No differences waiting for review." : "No findings match."}</td></tr>}
            {findings.body.data.map((x) => (
              <tr key={`${x.settlement_id}/${x.attempt_id}`}>
                <td>{dayYear(x.settled_on)}</td>
                <td>{groupLabel(x)}<div className="mono muted">{x.settlement_id}</div></td>
                <td className="num">{inr(x.payment_minor)}</td><td className="num">{inr(x.actual_minor)}</td><td className="num">{inr(x.expected_minor)}</td>
                <td><strong>{varianceText(x.variance_minor)}</strong>{x.tolerance_minor > 0 && <div className="muted">allowed {inr(x.tolerance_minor)}</div>}</td>
                <td>{x.case_status ? <Chip status={x.case_status} /> : <span className="muted">no case</span>}<div className="muted">{x.owner ?? "unassigned"}</div>{x.case_status && x.case_status !== "resolved" && <a href="/reconciliation">Open on Reconciliation</a>}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="muted" style={{ marginTop: 0 }}>{findings.body.note} Each difference is one review case on the Reconciliation page, where it can be assigned and resolved with a note.</p>

      <div style={{ display: "flex", gap: 16, alignItems: "center", flexWrap: "wrap", margin: "20px 0 32px" }}>
        {canWrite ? (
          <SettingsForm title="Run the check again" hint="Re-checks settlements against the rules as they are now: use it after adding or changing a rule, because old settlements are not re-checked by themselves. It opens or updates review cases and changes no money." action={runFeeCheck} button="Run the check" busy="Checking…" done="Done.">
            <label className="field">Only settlements on or after<input type="date" name="since" /><small className="muted" style={{ fontWeight: 400 }}>Empty checks the newest 200.</small></label>
          </SettingsForm>
        ) : <p className="muted">Running the check needs a finance or admin role.</p>}
      </div>

      <h2 id="rules" style={{ scrollMarginTop: 72 }}>Your fee rules</h2>
      <p className="muted" style={{ marginTop: 0, maxWidth: "70ch" }}>One rule per provider, method and start day. A payment is checked against the newest rule that had started on the day it was made, so a new rate needs a later start day and old payments keep their old rate.</p>
      <div className="tablewrap" style={{ marginBottom: 20 }}>
        <table>
          <caption className="sr">Fee rules you have entered</caption>
          <thead><tr><th scope="col">Provider and method</th><th scope="col">Rate</th><th scope="col">Refunds</th><th scope="col">From</th><th scope="col">Added by</th>{canWrite && <th scope="col"><span className="sr">Remove</span></th>}</tr></thead>
          <tbody>
            {rules.body.data.length === 0 && <tr><td colSpan={canWrite ? 6 : 5} className="empty">No rules yet.</td></tr>}
            {rules.body.data.map((x) => (
              <tr key={x.id}>
                <td>{providerLabel(x.provider)} · {methodLabel(x.method)}</td>
                <td>{ruleSentence(x)}{x.note && <div className="muted">{x.note}</div>}</td>
                <td>{x.refund_returns_fee ? "Provider gives the fee back" : "Fee is not given back"}</td>
                <td>{dayYear(x.effective_from)}</td><td className="muted">{x.created_by}</td>
                {canWrite && <td style={{ textAlign: "right" }}><ConfirmAct action={deleteFeeRule} id={x.id} label="Remove" question="Remove this rule? Findings already recorded keep a copy of it." yes="Remove" /></td>}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {canWrite && (
        <>
          <SettingsForm title="Add a fee rule" hint="Copy the rate from your contract or the provider's fee schedule. Leave a box empty if your contract has no such term." action={addFeeRule} button="Add rule" busy="Adding…" done="Rule added.">
            <label className="field">Provider<input name="provider" list="fee-providers" required autoComplete="off" /></label>
            <label className="field">Method<select name="method" required defaultValue=""><option value="" disabled>Choose…</option>{FEE_METHODS.map((m) => <option key={m} value={m}>{methodLabel(m)}</option>)}</select></label>
            <label className="field">Percentage of the payment (%)<input name="percent" inputMode="decimal" required /><small className="muted" style={{ fontWeight: 400 }}>Enter 0 if there is no percentage fee.</small></label>
            <label className="field">Flat fee per payment (₹)<input name="flat" inputMode="decimal" /></label>
            <label className="field">Most one fee can be (₹)<input name="cap" inputMode="decimal" /><small className="muted" style={{ fontWeight: 400 }}>Before GST. Empty means no limit.</small></label>
            <label className="field">GST on the fee (%)<input name="gst" inputMode="decimal" /><small className="muted" style={{ fontWeight: 400 }}>Empty if your contract has none.</small></label>
            <label className="field">Applies to payments from<input type="date" name="effective_from" required /></label>
            <label className="field">Note<input name="note" maxLength={200} /></label>
            <label className="field checkbox"><input type="checkbox" name="refund_returns_fee" /> The provider gives the fee back when a payment is refunded</label>
          </SettingsForm>
          <SettingsForm title="Tolerance and who looks at differences" hint="A fee may differ from the rule by up to the tolerance before it is flagged, which covers rounding. New findings are assigned to the owner." action={saveFeeSettings}>
            <label className="field">Tolerance per payment (₹)<input name="tolerance" inputMode="decimal" defaultValue={(set.tolerance_minor / 100).toFixed(2)} /></label>
            <label className="field">Owner (name or email)<input name="owner" maxLength={120} defaultValue={set.owner ?? ""} /></label>
          </SettingsForm>
        </>
      )}
      {!canWrite && <p className="muted">Tolerance is {inr(set.tolerance_minor)} per payment; differences are assigned to {set.owner ?? "nobody in particular"}. Changing rules or settings needs a finance or admin role.</p>}
    </>
  );
}
