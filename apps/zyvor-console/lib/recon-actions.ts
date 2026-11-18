"use server";
import { revalidatePath } from "next/cache";
import type { FormState } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { feeRuleBody, feeRunBody, feeSettingsBody, runSentence, type FeeRun } from "@/lib/feecheck";
import { importSentence, problemsOf, statementBody, type ImportDone, type Preview, type Problem } from "@/lib/bankrecon";
import { healthSettingsBody } from "@/lib/routehealth";
import { automationBody } from "@/lib/routeauto";
import { rest } from "@/lib/rest";
import { allows, type Role } from "@/lib/session";

const DENIED = { error: "Your role does not allow that." };

/** Who is signed in and may do this; null when not allowed. */
async function allowed(need: Role) {
  const s = await currentSession();
  return s && allows(s.r, need) ? s : null;
}
const digits = (f: FormData) => {
  const id = String(f.get("id") ?? "").trim();
  return /^\d{1,18}$/.test(id) ? id : null;
};
const word = (f: FormData) => {
  const id = String(f.get("id") ?? "").trim();
  return /^[A-Za-z0-9_-]{1,80}$/.test(id) ? id : null;
};

// ---------- fee checks ----------

export async function addFeeRule(_: FormState, f: FormData): Promise<FormState> {
  const s = await allowed("finance");
  if (!s) return DENIED;
  const p = feeRuleBody(f);
  if (!p.ok) return { error: p.error };
  const r = await (await rest(s.u)).post("/v1/fee_rules", p.body);
  if (!r.ok) return { error: r.code === "duplicate_rule" ? "There is already a rule for that provider and method starting on that day. Give the new rate a later start day." : r.message };
  revalidatePath("/fees");
  return { ok: "Rule added. Existing settlements are not re-checked until you run the check." };
}

export async function deleteFeeRule(_: FormState, f: FormData): Promise<FormState> {
  const s = await allowed("finance");
  const id = word(f);
  if (!s) return DENIED;
  if (!id) return { error: "That rule could not be found." };
  const r = await (await rest(s.u)).del(`/v1/fee_rules/${encodeURIComponent(id)}`);
  if (!r.ok) return { error: r.message };
  revalidatePath("/fees");
  return { ok: "Rule removed." };
}

export async function saveFeeSettings(_: FormState, f: FormData): Promise<FormState> {
  const s = await allowed("finance");
  if (!s) return DENIED;
  const p = feeSettingsBody(f);
  if (!p.ok) return { error: p.error };
  const r = await (await rest(s.u)).post("/v1/fee_checks/settings", p.body);
  if (!r.ok) return { error: r.message };
  revalidatePath("/fees");
  return { ok: "Saved." };
}

/** Re-check settlements against the rules as they are now. Opens or updates the review cases; moves no money. */
export async function runFeeCheck(_: FormState, f: FormData): Promise<FormState> {
  const s = await allowed("finance");
  if (!s) return DENIED;
  const p = feeRunBody(f);
  if (!p.ok) return { error: p.error };
  const r = await (await rest(s.u)).post<FeeRun>("/v1/fee_checks/run", p.body);
  if (!r.ok) return { error: r.message };
  revalidatePath("/fees");
  revalidatePath("/reconciliation");
  return { ok: runSentence(r.body) };
}

// ---------- bank statements ----------

export type PreviewState = { kind: "preview"; preview: Preview } | { kind: "import"; done: ImportDone; warnings: Problem[] } | { kind: "refused"; error: string; errors: Problem[] } | undefined;

/** Read the file and say what an import would do. Nothing is stored. */
export async function previewStatement(_: PreviewState, f: FormData): Promise<PreviewState> {
  const s = await allowed("viewer");
  if (!s) return { kind: "refused", error: DENIED.error, errors: [] };
  const p = statementBody(f);
  if (!p.ok) return { kind: "refused", error: p.error, errors: [] };
  const r = await (await rest(s.u)).post<Preview>("/v1/bank_statements/preview", p.body);
  if (!r.ok) return { kind: "refused", error: r.message, errors: [] };
  return { kind: "preview", preview: r.body };
}

/** Import it. All or nothing: if any line has a problem the API stores nothing and lists every problem. */
export async function importStatement(_: PreviewState, f: FormData): Promise<PreviewState> {
  const s = await allowed("finance");
  if (!s) return { kind: "refused", error: DENIED.error, errors: [] };
  const p = statementBody(f);
  if (!p.ok) return { kind: "refused", error: p.error, errors: [] };
  const r = await (await rest(s.u)).post<ImportDone>("/v1/bank_statements/import_csv", p.body);
  if (!r.ok) return { kind: "refused", error: r.message, errors: problemsOf(r.body) };
  for (const path of ["/bank-statements", "/reconciliation", "/"]) revalidatePath(path);
  return { kind: "import", done: r.body, warnings: r.body.warnings ?? [] };
}

async function decide(f: FormData, verb: "confirm" | "reject"): Promise<FormState> {
  const s = await allowed("finance");
  if (!s) return DENIED;
  const id = digits(f);
  if (!id) return { error: "That suggestion could not be found." };
  const r = await (await rest(s.u)).post(`/v1/bank_statements/matches/${id}/${verb}`, {});
  if (!r.ok) {
    if (r.code === "not_suggested") return { error: "Someone already decided this one. Reload the page to see the result." };
    if (r.code === "line_already_matched" || r.code === "subject_already_matched") return { error: "Either the bank line or the item it was paired with has since been matched to something else. Reload the page." };
    return { error: r.message };
  }
  for (const path of ["/bank-statements", "/reconciliation", "/"]) revalidatePath(path);
  return { ok: verb === "confirm" ? "Confirmed." : "Rejected. It will not be suggested again." };
}
export const confirmBankMatch = async (_: FormState, f: FormData) => decide(f, "confirm");
export const rejectBankMatch = async (_: FormState, f: FormData) => decide(f, "reject");

/** Look again at every unmatched line, for example after a settlement report arrived. */
export async function rematchBank(_: FormState, __: FormData): Promise<FormState> {
  const s = await allowed("finance");
  if (!s) return DENIED;
  const r = await (await rest(s.u)).post<{ settlements_rechecked: number; matched: { confirmed: number; suggested: number }; cases_opened: number }>("/v1/bank_statements/rematch", {});
  if (!r.ok) return { error: r.message };
  revalidatePath("/bank-statements");
  return { ok: `Looked again: ${r.body.matched.confirmed} matched by reference, ${r.body.matched.suggested} suggested for a person, ${r.body.cases_opened} case${r.body.cases_opened === 1 ? "" : "s"} opened.` };
}

/** Take a case. Same endpoint as on Reconciliation, but this page is refreshed afterwards. */
export async function takeBankCase(_: FormState, f: FormData): Promise<FormState> {
  const s = await allowed("finance");
  const id = word(f);
  if (!s) return DENIED;
  if (!id) return { error: "That case could not be found." };
  const r = await (await rest(s.u)).post(`/v1/recon/cases/${encodeURIComponent(id)}/assign`, { assignee: s.u });
  if (!r.ok) return { error: r.message };
  for (const path of ["/bank-statements", "/reconciliation", "/"]) revalidatePath(path);
  return { ok: "Assigned to you." };
}

// ---------- routing health ----------

export async function saveRoutingSettings(_: FormState, f: FormData): Promise<FormState> {
  const s = await allowed("admin");
  if (!s) return { error: "Only an admin can change these limits." };
  const p = healthSettingsBody(f);
  if (!p.ok) return { error: p.error };
  const r = await (await rest(s.u)).put("/v1/routing/health/settings", p.body);
  if (!r.ok) return { error: r.message };
  revalidatePath("/routing");
  return { ok: "Saved." };
}

export async function saveRoutingAutomation(_: FormState, f: FormData): Promise<FormState> {
  const s = await allowed("admin");
  if (!s) return { error: "Only an admin can change routing automation." };
  const p = automationBody(f);
  if (!p.ok) return { error: p.error };
  const r = await (await rest(s.u)).put("/v1/routing/automation", p.body);
  if (!r.ok) return { error: r.message };
  revalidatePath("/routing");
  return { ok: "Saved." };
}

/** The kill switch: `pause` stops every routing automation at once, `resume` lets each switch act again. */
export async function setRoutingKillSwitch(_: FormState, f: FormData): Promise<FormState> {
  const s = await allowed("admin");
  if (!s) return { error: "Only an admin can use the kill switch." };
  const resume = f.get("action") === "resume";
  const reason = String(f.get("reason") ?? "").trim().slice(0, 500);
  const r = await (await rest(s.u)).post(resume ? "/v1/routing/automation/resume" : "/v1/routing/automation/pause", resume ? {} : reason ? { reason } : {});
  if (!r.ok) return { error: r.message };
  revalidatePath("/routing");
  return { ok: resume ? "Routing automation can act again." : "Paused. No routing automation is acting." };
}
