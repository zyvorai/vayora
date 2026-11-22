// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

"use client";
import Link from "next/link";
import { startTransition, useActionState, useRef, useState, type FormEvent } from "react";
import { DATE_FORMATS, DELIMITERS, MAPPING_FIELDS, MAX_STATEMENT_BYTES, importSentence, problemText, type Layout, type Problem } from "@/lib/bankrecon";
import { dayYear, inr } from "@/lib/format";
import { importStatement, previewStatement } from "@/lib/recon-actions";

function Problems({ list, title }: { list: Problem[]; title: string }) {
  if (list.length === 0) return null;
  return (
    <div className="tablewrap" style={{ marginTop: 10 }}>
      <table>
        <caption className="sr">{title}</caption>
        <thead><tr><th scope="col">Line</th><th scope="col">Column</th><th scope="col">{title}</th></tr></thead>
        <tbody>{list.map((p, i) => <tr key={i}><td>{p.line > 0 ? p.line : "File"}</td><td>{p.column ?? "—"}</td><td>{p.message}</td></tr>)}</tbody>
      </table>
    </div>
  );
}

/** Upload a bank statement: choose a layout (or name the columns), check it without saving anything, then import. */
export function StatementUpload({ layouts, canImport }: { layouts: Layout[]; canImport: boolean }) {
  const form = useRef<HTMLFormElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const [csv, setCsv] = useState("");
  const [filename, setFilename] = useState("");
  const [readError, setReadError] = useState("");
  const [version, setVersion] = useState(0); // any change to the file or settings makes the last check stale
  const [checked, setChecked] = useState(-1);
  const [pState, preview, checking] = useActionState(previewStatement, undefined);
  const [iState, doImport, importing] = useActionState(importStatement, undefined);
  const touch = () => setVersion((v) => v + 1);

  const choose = async (file: File | undefined) => {
    setReadError("");
    if (!file) return;
    if (file.size > MAX_STATEMENT_BYTES) { setReadError("That file is too large to send in one go. Split it by month and upload each month."); return; }
    try {
      setCsv((await file.text()).replace(/^﻿/, ""));
      setFilename(file.name);
      touch();
    } catch {
      setReadError("The file could not be read. Try saving it as CSV again.");
    }
  };
  const data = () => {
    const fd = new FormData(form.current!);
    fd.set("csv", csv);
    fd.set("filename", filename);
    return fd;
  };
  const check = (e: FormEvent) => { e.preventDefault(); setChecked(version); const fd = data(); startTransition(() => preview(fd)); };
  const go = () => { const fd = data(); startTransition(() => doImport(fd)); };

  const p = pState?.kind === "preview" ? pState.preview : null;
  const fresh = p?.ok && checked === version;
  const lineCount = csv ? csv.split(/\r?\n/).length : 0;

  return (
    <div style={{ display: "grid", gap: 16, maxWidth: 860 }}>
      <form ref={form} onSubmit={check} onChange={touch} className="card" style={{ display: "grid", gap: 14 }}>
        <div>
          <h2 style={{ margin: 0 }}>Upload a bank statement</h2>
          <p className="muted" style={{ margin: "4px 0 0" }}>Download the statement from your bank as a CSV file. First choose <strong>Check the file</strong>: it reads the file and tells you what would happen, and saves nothing. Then import it.</p>
        </div>
        <div style={{ display: "grid", gap: 6 }}>
          <label htmlFor="stmt-file" className="f" style={{ margin: 0 }}>Statement file (CSV)</label>
          <input ref={fileInput} id="stmt-file" type="file" accept=".csv,.txt,text/csv,text/plain" onChange={(e) => { void choose(e.target.files?.[0]); }} />
          {readError && <span className="note bad" role="alert">{readError}</span>}
        </div>
        {filename ? (
          <p style={{ margin: 0 }}>Using <strong>{filename}</strong> ({lineCount.toLocaleString("en-IN")} lines). <button type="button" className="linkbtn" style={{ color: "var(--accent-text)", fontSize: ".875rem" }} onClick={() => { setCsv(""); setFilename(""); if (fileInput.current) fileInput.current.value = ""; touch(); }}>Remove the file</button></p>
        ) : (
          <label className="field">Or paste the statement text
            <textarea rows={5} spellCheck={false} value={csv} onChange={(e) => { setCsv(e.target.value); touch(); }} placeholder={"Date,Narration,Reference,Debit,Credit\n..."} />
          </label>
        )}
        <label className="field" style={{ maxWidth: 420 }}>Which bank or layout is it?
          <select name="layout" defaultValue="auto">
            {layouts.map((l) => <option key={l.name} value={l.name}>{l.name === "auto" ? "Work it out from the file (try this first)" : `${l.name} — ${l.about}`}</option>)}
          </select>
          <small className="muted" style={{ fontWeight: 400 }}>These layouts follow the column names banks commonly use. They have been tested on made-up files only, so always check a real file first.</small>
        </label>
        <details>
          <summary>The columns are named differently, or the dates look odd</summary>
          <div style={{ display: "grid", gap: 12, gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", marginTop: 12 }}>
            <p className="muted" style={{ gridColumn: "1 / -1", margin: 0 }}>Type a column&apos;s heading exactly as it is in the file (capitals and spaces do not matter), or <code>#3</code> for the third column. Leave a box empty to let the layout decide.</p>
            {MAPPING_FIELDS.map((m) => <label key={m.key} className="field">{m.label}<input name={`map_${m.key}`} autoComplete="off" /><small className="muted" style={{ fontWeight: 400 }}>{m.hint}</small></label>)}
            <label className="field">How dates are written<select name="date_format" defaultValue="">{DATE_FORMATS.map((d) => <option key={d.value} value={d.value}>{d.label}</option>)}</select><small className="muted" style={{ fontWeight: 400 }}>Dates are always read day first, never month first.</small></label>
            <label className="field">What separates the columns<select name="delimiter" defaultValue="">{DELIMITERS.map((d) => <option key={d.value} value={d.value}>{d.label}</option>)}</select></label>
          </div>
        </details>
        <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
          <button className="btn small" disabled={checking || importing}>{checking ? "Checking…" : "Check the file"}</button>
          {canImport && <button type="button" className="btn primary small" disabled={!fresh || importing || iState?.kind === "import"} onClick={go} aria-describedby="import-why">{importing ? "Importing…" : "Import"}</button>}
          <span id="import-why" className="muted">{!canImport ? "Importing needs a finance or admin role." : iState?.kind === "import" ? "Imported." : fresh ? "The file passed the check. Importing saves it." : "Check the file first; Import unlocks when it passes."}</span>
        </div>
      </form>

      <div role="status" aria-live="polite" style={{ display: "grid", gap: 12 }}>
        {pState?.kind === "refused" && <div className="note bad"><strong>Could not check the file.</strong> {pState.error}</div>}
        {p && !p.ok && (
          <div className="card">
            <p className="note bad" style={{ margin: 0 }}><strong>The file has problems, so it cannot be imported.</strong> Nothing was saved. Fix these in the file (or the layout above) and check it again.</p>
            <Problems list={p.errors} title="Problem" />
            <Problems list={p.warnings} title="Warning" />
          </div>
        )}
        {p?.ok && (
          <div className="card" style={{ display: "grid", gap: 12 }}>
            <h2 style={{ margin: 0 }}>What an import would do</h2>
            <p className="muted" style={{ margin: 0 }}>Nothing has been saved yet. Layout used: <strong>{p.layout}</strong>.</p>
            <div className="facts" style={{ marginBottom: 0 }}>
              <div><div className="k">Lines read</div><div className="v">{p.rows.toLocaleString("en-IN")}</div></div>
              <div><div className="k">New</div><div className="v">{(p.new_rows ?? 0).toLocaleString("en-IN")}</div></div>
              <div><div className="k">Already held (skipped)</div><div className="v">{(p.duplicate_rows ?? 0).toLocaleString("en-IN")}</div></div>
              <div><div className="k">Money in</div><div className="v">{inr(p.credits?.total_minor ?? 0)}</div><div className="muted">{p.credits?.count ?? 0} line{p.credits?.count === 1 ? "" : "s"}</div></div>
              <div><div className="k">Money out</div><div className="v">{inr(p.debits?.total_minor ?? 0)}</div><div className="muted">{p.debits?.count ?? 0} line{p.debits?.count === 1 ? "" : "s"}</div></div>
              <div><div className="k">Period</div><div className="v" style={{ fontSize: "1rem" }}>{p.period_from && p.period_to ? `${dayYear(p.period_from)} to ${dayYear(p.period_to)}` : "—"}</div></div>
            </div>
            {p.already_imported_as != null && <p className="note warn" style={{ margin: 0 }}>This exact file was imported before (import number {p.already_imported_as}). Importing again adds nothing new.</p>}
            {p.warnings.length > 0 && <div><strong>Worth a look (these do not stop the import)</strong><Problems list={p.warnings} title="Warning" /></div>}
            {p.sample && p.sample.length > 0 && (
              <div>
                <strong>The first lines as Zyvor read them.</strong> <span className="muted">Check that the dates and amounts are what you expect.</span>
                <div className="tablewrap" style={{ marginTop: 8 }}>
                  <table>
                    <thead><tr><th scope="col">Line</th><th scope="col">Date</th><th scope="col">Description</th><th scope="col">Reference</th><th scope="col" className="num">Amount</th></tr></thead>
                    <tbody>{p.sample.map((s) => <tr key={s.line}><td>{s.line}</td><td>{dayYear(s.posted_on)}</td><td style={{ maxWidth: 320 }}>{s.narration}</td><td className="mono">{s.reference ?? "—"}</td><td className="num">{s.amount_minor >= 0 ? "in " : "out "}{inr(Math.abs(s.amount_minor))}</td></tr>)}</tbody>
                  </table>
                </div>
              </div>
            )}
          </div>
        )}
        {iState?.kind === "refused" && (
          <div className="card">
            <p className="note bad" style={{ margin: 0 }}><strong>The statement was not imported.</strong> {iState.error} Nothing was saved.</p>
            <Problems list={iState.errors} title="Problem" />
          </div>
        )}
        {iState?.kind === "import" && (
          <div className="note ok">
            <strong>Imported.</strong> {importSentence(iState.done)}{" "}
            <Link href="/bank-statements?tab=review">Review the suggestions</Link> · <Link href="/bank-statements?tab=cases">See unmatched items</Link>
            {iState.warnings.length > 0 && <Problems list={iState.warnings} title="Warning" />}
          </div>
        )}
      </div>
    </div>
  );
}
