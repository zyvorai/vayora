import Link from "next/link";
import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { StatementUpload } from "@/components/StatementUpload";
import { Tabs } from "@/components/Tabs";
import { currentSession } from "@/lib/auth";
import { BANK_CASE_KINDS, SUGGESTED_LABEL, ageingRows, caseKindLabel, evidenceFacts, lineAmount, ruleLabel, subjectLabel, waitingText, type Ageing, type BankCase, type BankLine, type ImportRow, type Layout } from "@/lib/bankrecon";
import { dayYear, inr, istDateTime } from "@/lib/format";
import { confirmBankMatch, rejectBankMatch, rematchBank, takeBankCase } from "@/lib/recon-actions";
import { rest } from "@/lib/rest";
import { query } from "@/lib/restcall";
import { allows } from "@/lib/session";

type Search = { tab?: string; status?: string; direction?: string; import_id?: string; line?: string };
const TABS = [{ key: "review", label: "To review" }, { key: "upload", label: "Upload" }, { key: "lines", label: "All lines" }, { key: "imports", label: "Past uploads" }, { key: "cases", label: "Unmatched and ageing" }];

type LineDetail = BankLine & {
  matches: { id: number; subject_type: string; subject_id: string; status: string; rule: string; evidence: unknown; decided_by: string | null }[];
  cases: { id: string; kind: string; status: string; assignee: string | null }[];
};

function Evidence({ evidence }: { evidence: unknown }) {
  const facts = evidenceFacts(evidence);
  if (facts.length === 0) return null;
  return (
    <details style={{ marginTop: 4 }}>
      <summary>Why Zyvor thinks so</summary>
      <dl className="kv" style={{ marginTop: 8 }}>{facts.map((f) => <div key={f.label} style={{ display: "contents" }}><dt>{f.label}</dt><dd>{f.value}</dd></div>)}</dl>
    </details>
  );
}

const Money = ({ l }: { l: Pick<BankLine, "amount_minor"> }) => {
  const a = lineAmount(l);
  return <span><span className="muted">{a.direction === "in" ? "in " : "out "}</span>{a.text}</span>;
};

export default async function BankStatements({ searchParams }: { searchParams: Promise<Search> }) {
  const sp = await searchParams;
  const tab = TABS.some((t) => t.key === sp.tab) ? (sp.tab as string) : "review";
  const me = await currentSession();
  const canWrite = !!me && allows(me.r, "finance");
  const r = await rest();
  const head = (
    <>
      <div className="head"><div><h1>Bank statements</h1><p className="lede">Upload what your bank shows, and Zyvor pairs each line with a settlement, payout or refund. A pairing by reference is final. A pairing by amount and date is only a suggestion until a person confirms it.</p></div></div>
      <p className="muted" style={{ marginTop: 0, maxWidth: "70ch" }}>Built and tested on made-up statements and simulated providers. It has not been tried on a real bank export, so check a real file before importing it.</p>
      <Tabs base="/bank-statements" current={tab} tabs={TABS} />
    </>
  );
  const failed = (message: string) => <>{head}<p className="note bad" role="alert">{message}</p></>;

  if (tab === "upload") {
    const layouts = await r.get<{ data: Layout[] }>("/v1/bank_statements/layouts");
    if (!layouts.ok) return failed(layouts.message);
    return <>{head}<StatementUpload layouts={layouts.body.data} canImport={canWrite} /></>;
  }

  if (tab === "imports") {
    const imports = await r.get<{ data: ImportRow[] }>("/v1/bank_statements/imports");
    if (!imports.ok) return failed(imports.message);
    return (
      <>
        {head}
        <div className="tablewrap">
          <table>
            <caption className="sr">Statements uploaded so far</caption>
            <thead><tr><th scope="col">Uploaded</th><th scope="col">File</th><th scope="col">Period</th><th scope="col" className="num">Lines</th><th scope="col" className="num">New</th><th scope="col" className="num">Already held</th><th scope="col" className="num">Money in</th><th scope="col" className="num">Money out</th><th scope="col">By</th></tr></thead>
            <tbody>
              {imports.body.data.length === 0 && <tr><td colSpan={9} className="empty">No statements uploaded yet. <Link href="/bank-statements?tab=upload">Upload one</Link>.</td></tr>}
              {imports.body.data.map((i) => (
                <tr key={i.id}>
                  <td>{istDateTime(i.created_at)}</td>
                  <td>{i.filename ?? <span className="muted">no name</span>}<div className="muted">{i.source === "csv" ? `layout ${i.layout}` : i.source}</div></td>
                  <td>{i.period_from && i.period_to ? `${dayYear(i.period_from)} to ${dayYear(i.period_to)}` : "—"}</td>
                  <td className="num"><Link href={`/bank-statements?tab=lines&import_id=${i.id}`}>{i.rows}</Link></td><td className="num">{i.added}</td><td className="num">{i.duplicates}</td>
                  <td className="num">{inr(i.credits_minor)}</td><td className="num">{inr(i.debits_minor)}</td><td className="muted">{i.created_by}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </>
    );
  }

  if (tab === "cases") {
    const [ageing, cases] = await Promise.all([r.get<Ageing>("/v1/recon/ageing"), r.get<{ data: BankCase[] }>("/v1/recon/cases?limit=200")]);
    if (!ageing.ok) return failed(ageing.message);
    if (!cases.ok) return failed(cases.message);
    const kinds = new Set<string>(BANK_CASE_KINDS);
    const open = cases.body.data.filter((c) => kinds.has(c.kind) && c.status !== "resolved");
    const rows = ageingRows(ageing.body).filter((x) => kinds.has(x.kind));
    return (
      <>
        {head}
        <h2>How long open items have waited</h2>
        <p className="muted" style={{ marginTop: 0 }}>Counts of open cases by age in days. A statement nobody has uploaded yet is never treated as a missing credit.</p>
        <div className="tablewrap" style={{ marginBottom: 24 }}>
          <table>
            <caption className="sr">Open bank cases by how long they have waited</caption>
            <thead><tr><th scope="col">Kind</th>{ageing.body.bucket_days.map((b) => <th scope="col" key={b} className="num">{b} days</th>)}<th scope="col" className="num">Nobody has it</th><th scope="col" className="num">Oldest</th><th scope="col" className="num">Amount</th></tr></thead>
            <tbody>
              {rows.length === 0 && <tr><td colSpan={ageing.body.bucket_days.length + 4} className="empty">No open bank cases.</td></tr>}
              {rows.map((x) => <tr key={x.kind}><td>{caseKindLabel(x.kind)}</td>{x.buckets.map((n, i) => <td key={i} className="num">{n}</td>)}<td className="num">{x.unassigned}</td><td className="num">{x.oldest_days} d</td><td className="num">{inr(x.amount_minor)}</td></tr>)}
            </tbody>
          </table>
        </div>
        <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap", marginBottom: 12 }}>
          <h2 style={{ margin: 0 }}>Open items ({open.length})</h2>
          {canWrite && <Act action={rematchBank} label="Look for matches again" showOk />}
        </div>
        <div className="tablewrap">
          <table>
            <caption className="sr">Open bank cases</caption>
            <thead><tr><th scope="col">What</th><th scope="col">Details</th><th scope="col" className="num">Amount</th><th scope="col">Waiting</th><th scope="col">Status</th><th scope="col">Owner</th>{canWrite && <th scope="col"><span className="sr">Action</span></th>}</tr></thead>
            <tbody>
              {open.length === 0 && <tr><td colSpan={canWrite ? 7 : 6} className="empty">Nothing is waiting. Every line is matched, or no statement covers the period yet.</td></tr>}
              {open.map((c) => (
                <tr key={c.id}>
                  <td>{caseKindLabel(c.kind)}</td>
                  <td style={{ maxWidth: 420 }}>{c.explanation}{c.subject_type === "bank_line" && <div><Link href={`/bank-statements?tab=lines&line=${encodeURIComponent(c.subject_id)}`}>See the bank line</Link></div>}</td>
                  <td className="num">{c.actual_minor !== null ? inr(c.actual_minor) : c.expected_minor !== null ? inr(c.expected_minor) : "—"}</td>
                  <td>{waitingText(c.age_days)}</td><td><Chip status={c.status} /></td><td>{c.assignee ?? <span className="muted">unassigned</span>}</td>
                  {canWrite && <td>{c.status === "open" && <Act action={takeBankCase} id={c.id} label="Take it" />}</td>}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="muted">Bank debits that nothing explains do not become cases (a statement holds many charges that are not Zyvor&apos;s). Find them under <Link href="/bank-statements?tab=lines&status=unmatched&direction=debit">All lines</Link>.</p>
      </>
    );
  }

  if (tab === "lines") {
    const status = ["unmatched", "suggested", "matched"].includes(sp.status ?? "") ? sp.status : undefined;
    const direction = ["credit", "debit"].includes(sp.direction ?? "") ? sp.direction : undefined;
    const importId = /^\d{1,18}$/.test(sp.import_id ?? "") ? sp.import_id : undefined;
    const lineId = /^\d{1,18}$/.test(sp.line ?? "") ? sp.line : undefined;
    const [lines, one] = await Promise.all([
      r.get<{ data: BankLine[] }>(`/v1/bank_statements/lines${query({ status, direction, import_id: importId, limit: 100 })}`),
      lineId ? r.get<LineDetail>(`/v1/bank_statements/lines/${lineId}`) : Promise.resolve(null),
    ]);
    if (!lines.ok) return failed(lines.message);
    const link = (over: Record<string, string | undefined>) => `/bank-statements${query({ tab: "lines", status, direction, import_id: importId, ...over })}`;
    return (
      <>
        {head}
        <nav aria-label="Filter lines" style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center", marginBottom: 14 }}>
          <div className="seg">{[["", "All"], ["unmatched", "Unmatched"], ["suggested", "Suggested"], ["matched", "Matched"]].map(([k, l]) => <Link key={k} href={link({ status: k || undefined, line: undefined })} aria-current={(status ?? "") === k}>{l}</Link>)}</div>
          <div className="seg">{[["", "In and out"], ["credit", "Money in"], ["debit", "Money out"]].map(([k, l]) => <Link key={k} href={link({ direction: k || undefined, line: undefined })} aria-current={(direction ?? "") === k}>{l}</Link>)}</div>
          {importId && <span className="muted">Only upload number {importId}. <Link href={link({ import_id: undefined })}>Show all</Link></span>}
        </nav>
        {one && (!one.ok ? <p className="note bad" role="alert">{one.message}</p> : (
          <section className="card" style={{ marginBottom: 16 }} aria-label="Bank line detail">
            <h2 style={{ marginTop: 0 }}>Line {one.body.id}: {dayYear(one.body.posted_on)}, <Money l={one.body} /></h2>
            <p style={{ margin: "0 0 8px" }}>{one.body.narration}{one.body.reference ? <> · reference <span className="mono">{one.body.reference}</span></> : null}</p>
            {one.body.matches.length === 0 && <p className="muted">Nothing has been paired with this line.</p>}
            {one.body.matches.map((m) => (
              <div key={m.id} style={{ borderTop: "1px solid var(--line)", padding: "10px 0" }}>
                <strong>{subjectLabel(m.subject_type, m.subject_id)}</strong> {m.status === "suggested" ? <span className="chip warn">suggested</span> : <Chip status={m.status === "confirmed" ? "paid" : "rejected"} />}{" "}
                <span className="muted">{m.status === "confirmed" ? "confirmed" : m.status}: {ruleLabel(m.rule)}{m.decided_by ? ` · decided by ${m.decided_by === "system" ? "Zyvor (by reference)" : m.decided_by}` : ""}</span>
                <Evidence evidence={m.evidence} />
              </div>
            ))}
            {one.body.cases.length > 0 && <p className="muted" style={{ marginBottom: 0 }}>Cases: {one.body.cases.map((c) => `${caseKindLabel(c.kind)} (${c.status})`).join(", ")}</p>}
          </section>
        ))}
        <div className="tablewrap">
          <table>
            <caption className="sr">Bank statement lines</caption>
            <thead><tr><th scope="col">Date</th><th scope="col">Description</th><th scope="col" className="num">Amount</th><th scope="col">Status</th><th scope="col">Paired with</th></tr></thead>
            <tbody>
              {lines.body.data.length === 0 && <tr><td colSpan={5} className="empty">No lines here. <Link href="/bank-statements?tab=upload">Upload a statement</Link> to see them.</td></tr>}
              {lines.body.data.map((l) => (
                <tr key={l.id}>
                  <td style={{ whiteSpace: "nowrap" }}>{dayYear(l.posted_on)}</td>
                  <td style={{ maxWidth: 380 }}>{l.narration}{l.reference && <div className="mono muted">{l.reference}</div>}</td>
                  <td className="num"><Money l={l} /></td>
                  <td>{l.status === "suggested" ? <span className="chip warn">suggested</span> : l.status === "matched" ? <span className="chip ok">matched</span> : <span className="chip">unmatched</span>}</td>
                  <td>
                    {l.match ? <>{subjectLabel(l.match.subject_type, l.match.subject_id)}<div className="muted">{ruleLabel(l.match.rule)}{l.match.confirmed_by ? ` · ${l.match.confirmed_by === "system" ? "by reference" : `confirmed by ${l.match.confirmed_by}`}` : ""}</div></>
                      : l.suggestions.length > 0 ? <span className="muted">{l.suggestions.length} suggestion{l.suggestions.length === 1 ? "" : "s"} waiting for a person</span> : <span className="muted">nothing yet</span>}
                    <div><Link href={link({ line: String(l.id) })}>Details</Link></div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="muted">Showing up to 100 lines, newest first. Narrow the list with the filters above.</p>
      </>
    );
  }

  // review: suggestions waiting for a person
  const lines = await r.get<{ data: BankLine[] }>("/v1/bank_statements/lines?status=suggested&limit=100");
  if (!lines.ok) return failed(lines.message);
  return (
    <>
      {head}
      <h2>Suggestions waiting for a person ({lines.body.data.length})</h2>
      <p className="muted" style={{ marginTop: 0, maxWidth: "70ch" }}>No reference tied these lines to anything, so Zyvor can only guess from the amount and the date, or a reference matched but the amount differs. <strong>{SUGGESTED_LABEL}.</strong> Nothing is final until someone confirms it. Rejecting a suggestion means it is not suggested again.</p>
      {lines.body.data.length === 0 && <div className="card empty">Nothing to review. <Link href="/bank-statements?tab=upload">Upload a statement</Link> or look at <Link href="/bank-statements?tab=cases">unmatched items</Link>.</div>}
      <div style={{ display: "grid", gap: 14 }}>
        {lines.body.data.map((l) => (
          <section key={l.id} className="card" aria-label={`Bank line ${l.id}`}>
            <div style={{ display: "flex", gap: 12, justifyContent: "space-between", flexWrap: "wrap" }}>
              <div><strong>{dayYear(l.posted_on)} · <Money l={l} /></strong><div>{l.narration}</div>{l.reference && <div className="mono muted">{l.reference}</div>}</div>
              <span className="chip warn">{SUGGESTED_LABEL}</span>
            </div>
            {l.suggestions.length > 1 && <p className="note warn" style={{ margin: "10px 0 0" }}>There is more than one possibility. Confirm the right one and reject the others.</p>}
            {l.suggestions.map((s) => (
              <div key={s.id} style={{ borderTop: "1px solid var(--line)", marginTop: 12, paddingTop: 12, display: "flex", gap: 12, justifyContent: "space-between", flexWrap: "wrap", alignItems: "start" }}>
                <div style={{ maxWidth: 520 }}>
                  <div>Could be <strong>{subjectLabel(s.subject_type, s.subject_id)}</strong></div>
                  <div className="muted">{ruleLabel(s.rule)}</div>
                  <Evidence evidence={s.evidence} />
                </div>
                {canWrite ? (
                  <span style={{ display: "inline-flex", gap: 8, flexWrap: "wrap" }}>
                    <Act action={confirmBankMatch} id={String(s.id)} label="Confirm: this is it" primary showOk />
                    <Act action={rejectBankMatch} id={String(s.id)} label="Reject" showOk />
                  </span>
                ) : <span className="muted">A finance or admin user decides.</span>}
              </div>
            ))}
          </section>
        ))}
      </div>
    </>
  );
}
