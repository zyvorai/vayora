import { beforeEach, describe, expect, it, vi } from "vitest";

const session = vi.hoisted(() => ({ current: null as null | { u: string; r: string } }));
const calls = vi.hoisted(() => ({ list: [] as { method: string; path: string; body?: unknown }[], reply: new Map<string, unknown>() }));
const revalidated = vi.hoisted(() => [] as string[]);

vi.mock("next/cache", () => ({ revalidatePath: (p: string) => { revalidated.push(p); } }));
vi.mock("@/lib/auth", () => ({ currentSession: async () => session.current }));
vi.mock("@/lib/rest", () => {
  const go = async (method: string, path: string, body?: unknown) => {
    calls.list.push({ method, path, body });
    return (calls.reply.get(`${method} ${path}`) ?? { ok: true, status: 200, body: {} }) as unknown;
  };
  return { rest: async () => ({ get: (p: string) => go("GET", p), post: (p: string, b?: unknown) => go("POST", p, b), put: (p: string, b: unknown) => go("PUT", p, b), del: (p: string) => go("DELETE", p) }) };
});

import { addFeeRule, confirmBankMatch, deleteFeeRule, importStatement, previewStatement, rejectBankMatch, runFeeCheck, saveFeeSettings, saveRoutingAutomation, saveRoutingSettings, setRoutingKillSwitch, takeBankCase } from "./recon-actions";

const form = (o: Record<string, string>) => { const f = new FormData(); for (const [k, v] of Object.entries(o)) f.set(k, v); return f; };
const refused = (status: number, code: string, message: string, body: unknown = null) => ({ ok: false, status, code, message, body });
const rule = { provider: "razorpay", method: "upi", percent: "1.5", effective_from: "2026-04-01" };
const health = { window_attempts: "100", window_minutes: "30", min_samples: "10", degrade_failure_pct: "50", recover_failure_pct: "20", cooldown_seconds: "120", breaker_enabled: "on" };

beforeEach(() => { calls.list.length = 0; calls.reply.clear(); revalidated.length = 0; session.current = { u: "asha", r: "finance" }; });

describe("who may do what", () => {
  it("refuses every write without a signed-in person, before calling the API", async () => {
    session.current = null;
    expect(await addFeeRule(undefined, form(rule))).toEqual({ error: "Your role does not allow that." });
    expect(await confirmBankMatch(undefined, form({ id: "4" }))).toEqual({ error: "Your role does not allow that." });
    expect(calls.list).toEqual([]);
  });
  it("lets a viewer read and preview, but not write", async () => {
    session.current = { u: "vik", r: "viewer" };
    expect(await addFeeRule(undefined, form(rule))).toHaveProperty("error");
    expect(await deleteFeeRule(undefined, form({ id: "fr_1" }))).toHaveProperty("error");
    expect(await runFeeCheck(undefined, form({}))).toHaveProperty("error");
    expect(await rejectBankMatch(undefined, form({ id: "4" }))).toHaveProperty("error");
    expect(await importStatement(undefined, form({ csv: "a" }))).toMatchObject({ kind: "refused" });
    expect(calls.list).toEqual([]);
    calls.reply.set("POST /v1/bank_statements/preview", { ok: true, status: 200, body: { ok: true, rows: 1, errors: [], warnings: [] } });
    expect(await previewStatement(undefined, form({ csv: "Date,Narration,Credit\n01/10/2026,x,5" }))).toMatchObject({ kind: "preview" });
  });
  it("lets only an admin change routing limits", async () => {
    expect(await saveRoutingSettings(undefined, form(health))).toEqual({ error: "Only an admin can change these limits." });
    expect(calls.list).toEqual([]);
    session.current = { u: "root", r: "admin" };
    expect(await saveRoutingSettings(undefined, form(health))).toEqual({ ok: "Saved." });
    expect(calls.list).toEqual([{ method: "PUT", path: "/v1/routing/health/settings", body: { breaker_enabled: true, failover_enabled: false, window_attempts: 100, window_minutes: 30, min_samples: 10, degrade_failure_pct: 50, recover_failure_pct: 20, cooldown_seconds: 120 } }]);
  });
});

describe("routing automation", () => {
  const auto = { soft_retry_max_attempts: "2", soft_retry_window_seconds: "600", cost_success_floor_pct: "80", cost_premium_pct: "5", cost_min_fee_samples: "30", cost_aware_enabled: "on" };
  it("lets only an admin change it or pull the kill switch", async () => {
    expect(await saveRoutingAutomation(undefined, form(auto))).toEqual({ error: "Only an admin can change routing automation." });
    expect(await setRoutingKillSwitch(undefined, form({ action: "pause" }))).toEqual({ error: "Only an admin can use the kill switch." });
    expect(calls.list).toEqual([]);
    session.current = { u: "root", r: "admin" };
    expect(await saveRoutingAutomation(undefined, form(auto))).toEqual({ ok: "Saved." });
    expect(calls.list[0]).toEqual({ method: "PUT", path: "/v1/routing/automation", body: { shadow_enabled: false, soft_retry_enabled: false, cost_aware_enabled: true, soft_retry_max_attempts: 2, soft_retry_window_seconds: 600, cost_success_floor_bps: 8000, cost_premium_bps: 500, cost_min_fee_samples: 30 } });
    expect(revalidated).toContain("/routing");
  });
  it("pauses with a reason and resumes, and does not call the API for a bad form", async () => {
    session.current = { u: "root", r: "admin" };
    expect(await setRoutingKillSwitch(undefined, form({ action: "pause", reason: "  provider talks " }))).toHaveProperty("ok");
    expect(await setRoutingKillSwitch(undefined, form({ action: "resume" }))).toEqual({ ok: "Routing automation can act again." });
    expect(calls.list).toEqual([{ method: "POST", path: "/v1/routing/automation/pause", body: { reason: "provider talks" } }, { method: "POST", path: "/v1/routing/automation/resume", body: {} }]);
    calls.list.length = 0;
    expect(await saveRoutingAutomation(undefined, form({ ...auto, soft_retry_max_attempts: "9" }))).toEqual({ error: "Most attempts per payment: enter a whole number from 2 to 5." });
    expect(calls.list).toEqual([]);
  });
});

describe("fee checks", () => {
  it("adds a rule in the API's units and tells the person old settlements need a re-check", async () => {
    const r = await addFeeRule(undefined, form({ ...rule, flat: "2", gst: "18" }));
    expect(r?.ok).toContain("not re-checked until you run the check");
    expect(calls.list[0]).toEqual({ method: "POST", path: "/v1/fee_rules", body: { provider: "razorpay", method: "upi", bps: 150, flat_minor: 200, gst_bps: 1800, refund_returns_fee: false, effective_from: "2026-04-01" } });
    expect(revalidated).toContain("/fees");
  });
  it("explains a duplicate rule in words and does not call the API for a bad form", async () => {
    calls.reply.set("POST /v1/fee_rules", refused(409, "duplicate_rule", "a rule for that provider..."));
    expect((await addFeeRule(undefined, form(rule)))?.error).toContain("later start day");
    calls.list.length = 0;
    expect((await addFeeRule(undefined, form({ ...rule, percent: "" })))?.error).toContain("percentage");
    expect(calls.list).toEqual([]);
  });
  it("removes a rule by id and refuses an id that is not one", async () => {
    expect(await deleteFeeRule(undefined, form({ id: "fr_abc123" }))).toEqual({ ok: "Rule removed." });
    expect(calls.list[0]).toMatchObject({ method: "DELETE", path: "/v1/fee_rules/fr_abc123" });
    expect(await deleteFeeRule(undefined, form({ id: "../x" }))).toHaveProperty("error");
  });
  it("saves tolerance as paise and the owner", async () => {
    await saveFeeSettings(undefined, form({ tolerance: "0.50", owner: "asha" }));
    expect(calls.list[0]).toEqual({ method: "POST", path: "/v1/fee_checks/settings", body: { tolerance_minor: 50, owner: "asha" } });
  });
  it("runs the check and reports what the API counted", async () => {
    calls.reply.set("POST /v1/fee_checks/run", { ok: true, status: 200, body: { settlements_checked: 3, flagged: 1, ok: 1, no_rule: 1 } });
    const r = await runFeeCheck(undefined, form({ since: "2026-10-01" }));
    expect(r?.ok).toBe("Checked 3 settlements: 1 with a difference, 1 matching your rates, 1 with no rule to compare against.");
    expect(calls.list[0]).toMatchObject({ path: "/v1/fee_checks/run", body: { since: "2026-10-01" } });
  });
  it("shows the API's refusal", async () => {
    calls.reply.set("POST /v1/fee_checks/run", refused(403, "forbidden", "Your sign-in is not allowed to do that."));
    expect(await runFeeCheck(undefined, form({}))).toEqual({ error: "Your sign-in is not allowed to do that." });
  });
});

describe("bank statements", () => {
  const csv = "Date,Narration,Credit\n01/10/2026,x,5";
  it("a preview that finds problems is a result with the line-level errors, not a crash", async () => {
    calls.reply.set("POST /v1/bank_statements/preview", { ok: true, status: 200, body: { ok: false, layout: "auto", rows: 0, errors: [{ line: 2, column: "Date", message: "bad" }], warnings: [] } });
    const r = await previewStatement(undefined, form({ csv }));
    expect(r).toMatchObject({ kind: "preview", preview: { ok: false, errors: [{ line: 2 }] } });
    expect(calls.list[0].body).toEqual({ csv, layout: "auto" });
  });
  it("an import the API refuses brings back every problem and says nothing was saved", async () => {
    const body = { error: { code: "invalid_statement", message: "the statement was not imported: line 2: bad", errors: [{ line: 2, column: "Date", message: "bad" }, { line: 5, column: "Credit", message: "no" }] } };
    calls.reply.set("POST /v1/bank_statements/import_csv", refused(422, "invalid_statement", "the statement was not imported: line 2: bad", body));
    const r = await importStatement(undefined, form({ csv, layout: "hdfc", filename: "oct.csv" }));
    expect(r).toMatchObject({ kind: "refused", errors: [{ line: 2 }, { line: 5 }] });
    expect(revalidated).toEqual([]);
    expect(calls.list[0].body).toEqual({ csv, filename: "oct.csv", layout: "hdfc" });
  });
  it("a successful import returns the counts and refreshes the pages", async () => {
    calls.reply.set("POST /v1/bank_statements/import_csv", { ok: true, status: 200, body: { import_id: 7, rows: 2, added: 2, duplicates: 0, settlements_rechecked: 0, matched: { confirmed: 1, suggested: 1 }, cases_opened: 0, warnings: [{ line: 3, message: "balance" }] } });
    const r = await importStatement(undefined, form({ csv }));
    expect(r).toMatchObject({ kind: "import", done: { import_id: 7 }, warnings: [{ line: 3 }] });
    expect(revalidated).toContain("/bank-statements");
  });
  it("an empty file is refused here, with a sentence", async () => {
    expect(await previewStatement(undefined, form({ csv: "" }))).toMatchObject({ kind: "refused", error: "Choose a statement file, or paste its text." });
    expect(calls.list).toEqual([]);
  });
  it("confirm and reject call the match's own endpoint, and only with a numeric id", async () => {
    expect(await confirmBankMatch(undefined, form({ id: "12" }))).toEqual({ ok: "Confirmed." });
    expect(await rejectBankMatch(undefined, form({ id: "12" }))).toEqual({ ok: "Rejected. It will not be suggested again." });
    expect(calls.list.map((c) => c.path)).toEqual(["/v1/bank_statements/matches/12/confirm", "/v1/bank_statements/matches/12/reject"]);
    expect(await confirmBankMatch(undefined, form({ id: "12/../../x" }))).toHaveProperty("error");
    expect(calls.list).toHaveLength(2);
  });
  it("explains a suggestion someone else already decided", async () => {
    calls.reply.set("POST /v1/bank_statements/matches/12/confirm", refused(409, "not_suggested", "this match is confirmed"));
    expect((await confirmBankMatch(undefined, form({ id: "12" })))?.error).toContain("already decided");
    calls.reply.set("POST /v1/bank_statements/matches/13/confirm", refused(409, "line_already_matched", "taken"));
    expect((await confirmBankMatch(undefined, form({ id: "13" })))?.error).toContain("since been matched");
  });
  it("takes a case as the signed-in person", async () => {
    await takeBankCase(undefined, form({ id: "case_9" }));
    expect(calls.list[0]).toEqual({ method: "POST", path: "/v1/recon/cases/case_9/assign", body: { assignee: "asha" } });
  });
});
