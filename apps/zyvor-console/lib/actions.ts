"use server";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { revalidatePath } from "next/cache";
import { ZyvorError, type Zyvor } from "@zyvor/node";
import { currentSession } from "@/lib/auth";
import { COOKIE, SESSION_TTL_SECONDS, allows, createSession, passwordMatches, type Role, type Session } from "@/lib/session";
import { api, authConfig } from "@/lib/zyvor";

export type FormState = { ok?: string; error?: string; otp?: boolean; secret?: string; uri?: string; codes?: string[] } | undefined;

async function start(who: Omit<Session, "exp">) {
  (await cookies()).set(COOKIE, await createSession(authConfig().secret, who), {
    httpOnly: true,
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
    path: "/",
    maxAge: SESSION_TTL_SECONDS,
  });
}

export async function login(_: FormState, form: FormData): Promise<FormState> {
  const { username, password, secret } = authConfig();
  const u = String(form.get("username") ?? "").trim().toLowerCase();
  const p = String(form.get("password") ?? "");
  const otp = String(form.get("otp") ?? "").trim();
  let signedIn: Omit<Session, "exp"> | null = null;
  try {
    const r = await api().consoleUsers.authenticate({ username: u, password: p, otp: otp || undefined });
    signedIn = { i: r.user.id, u: r.user.username, r: r.user.role, m: r.mfa_enrolled };
  } catch (e) {
    if (!(e instanceof ZyvorError)) return { error: "Sign-in is unavailable right now. Try again in a moment." };
    if (e.code === "otp_required") return { otp: true };
    if (e.status === 423) return { error: "Too many failed attempts. Try again in 15 minutes, or ask an admin to unlock you." };
    if (e.status !== 401) return { error: "Sign-in is unavailable right now. Try again in a moment." };
  }
  if (!signedIn) {
    // Break-glass: the deployment's own admin may sign in only while no named user exists yet.
    // Check both fields every time so a wrong username costs the same as a wrong password.
    const userOk = await passwordMatches(secret, u, username.toLowerCase());
    const passOk = await passwordMatches(secret, p, password);
    let noUsers = false;
    if (userOk && passOk) {
      try {
        noUsers = (await api().consoleUsers.list()).data.length === 0;
      } catch {
        noUsers = false;
      }
    }
    if (userOk && passOk && noUsers) signedIn = { i: "bootstrap", u: username, r: "admin", m: false };
  }
  if (!signedIn) {
    await new Promise((r) => setTimeout(r, 500)); // slow down guessing
    return { error: "Incorrect username, password or code." };
  }
  await start(signedIn);
  const next = String(form.get("next") ?? "/");
  // Only same-site relative paths; never an open redirect.
  redirect(next.startsWith("/") && !next.startsWith("//") ? next : "/");
}

export async function logout() {
  (await cookies()).delete(COOKIE);
  redirect("/login");
}

const DENIED = { error: "Your role does not allow that." };

/** Run an API call from a form as the signed-in person. A refusal (state changed, not allowed) is shown, not thrown. */
async function run(paths: string[], fn: (z: Zyvor, s: Session) => Promise<unknown>, need: Role = "finance"): Promise<FormState> {
  const s = await currentSession();
  if (!s || !allows(s.r, need)) return DENIED;
  try {
    await fn(api(s.u), s);
  } catch (e) {
    if (e instanceof ZyvorError) return { error: e.message };
    throw e;
  }
  for (const p of paths) revalidatePath(p);
  return { ok: "Done." };
}
const id = (f: FormData) => String(f.get("id") ?? "");

export const approveRefund = async (_: FormState, f: FormData) => run(["/refunds", "/"], (z) => z.refunds.approve(id(f)));
export const rejectRefund = async (_: FormState, f: FormData) => run(["/refunds", "/"], (z) => z.refunds.reject(id(f)));
export const retryRefund = async (_: FormState, f: FormData) => run(["/refunds"], (z) => z.refunds.retry(id(f)));
export const assignCase = async (_: FormState, f: FormData) => run(["/reconciliation", "/"], (z, s) => z.cases.assign(id(f), String(f.get("assignee") ?? "").trim() || s.u));
export const resolveCase = async (_: FormState, f: FormData) => {
  const note = String(f.get("resolution") ?? "").trim();
  if (!note) return { error: "Say how it was resolved." };
  return run(["/reconciliation", "/"], (z) => z.cases.resolve(id(f), note));
};
export const replayEvent = async (_: FormState, f: FormData) => run(["/webhooks"], (z) => z.webhooks.replay(id(f)));
export const runBilling = async (_: FormState): Promise<FormState> => {
  let made = 0;
  const r = await run(["/billing"], async (z) => {
    made = (await z.billing.run()).invoices_created;
  });
  return r?.error ? r : { ok: `${made} invoice(s) created.` };
};
export const remind = async (_: FormState, f: FormData) => run(["/collections"], (z) => z.collections.remind(id(f), { channel: String(f.get("channel") ?? "whatsapp") as "whatsapp" }));
export const completeMilestone = async (_: FormState, f: FormData) => run(["/billing"], (z) => z.milestones.complete(id(f)));
export const resolveDispute = async (_: FormState, f: FormData) => run(["/disputes", "/"], (z) => z.disputes.resolve(id(f), String(f.get("outcome")) === "won" ? "won" : "lost"));

export const createLink = async (_: FormState, f: FormData): Promise<FormState> => {
  const rupees = Number(String(f.get("amount") ?? "").replace(/,/g, ""));
  const maxUses = String(f.get("max_uses") ?? "").trim();
  if (!Number.isFinite(rupees) || rupees <= 0) return { error: "Enter an amount greater than zero." };
  let path = "";
  const r = await run(["/links"], async (z) => {
    path = (await z.paymentLinks.create({
      description: String(f.get("description") ?? "").trim() || undefined,
      amountMinor: Math.round(rupees * 100),
      maxUses: maxUses ? Number(maxUses) : undefined,
      convenienceFeeBp: String(f.get("fee") ?? "").trim() ? Math.round(Number(String(f.get("fee")).replace(/,/g, "")) * 100) : undefined,
    })).path;
  });
  return r?.error ? r : { ok: path };
};
export const cancelLink = async (_: FormState, f: FormData) => run(["/links"], (z) => z.paymentLinks.cancel(id(f)));

// ---- people and security (admin) ----
export const createUser = async (_: FormState, f: FormData) =>
  run(["/users"], (z) => z.consoleUsers.create({ username: String(f.get("username") ?? ""), password: String(f.get("password") ?? ""), role: String(f.get("role") ?? "viewer") as Role }), "admin");
export const disableUser = async (_: FormState, f: FormData) => run(["/users"], (z, s) => (id(f) === s.i ? Promise.reject(new ZyvorError(422, "self", "You cannot disable yourself.")) : z.consoleUsers.disable(id(f))), "admin");
export const enableUser = async (_: FormState, f: FormData) => run(["/users"], (z) => z.consoleUsers.enable(id(f)), "admin");
export const resetUserMfa = async (_: FormState, f: FormData) => run(["/users"], (z) => z.consoleUsers.mfaReset(id(f)), "admin");

// ---- your own second factor (any signed-in named user) ----
export async function beginMfa(): Promise<FormState> {
  const s = await currentSession();
  if (!s || s.i === "bootstrap") return { error: "Create a named user first; the deployment admin cannot enrol a device." };
  try {
    const r = await api(s.u).consoleUsers.mfaBegin(s.i);
    return { secret: r.secret, uri: r.otpauth_uri };
  } catch (e) {
    if (e instanceof ZyvorError) return { error: e.message };
    throw e;
  }
}

export async function confirmMfa(_: FormState, f: FormData): Promise<FormState> {
  const s = await currentSession();
  if (!s || s.i === "bootstrap") return DENIED;
  try {
    const r = await api(s.u).consoleUsers.mfaConfirm(s.i, String(f.get("otp") ?? "").trim());
    await start({ i: s.i, u: s.u, r: s.r, m: true });
    return { codes: r.recovery_codes };
  } catch (e) {
    if (e instanceof ZyvorError) return { error: e.message };
    throw e;
  }
}

// ---- reminder drafts: a person approves wording before anything is sent ----
export const draftReminder = async (_: FormState, f: FormData) =>
  run(["/collections"], (z) => z.collections.draft(id(f), { channel: "whatsapp", ai: String(f.get("ai") ?? "") === "1" }));
export const approveDraft = async (_: FormState, f: FormData) =>
  run(["/collections"], (z) => z.collections.approveDraft(id(f), String(f.get("message") ?? "").trim() || undefined));
export const rejectDraft = async (_: FormState, f: FormData) => run(["/collections"], (z) => z.collections.rejectDraft(id(f)));

// ---- payouts: a second person approves, then the transfer is recorded with its bank reference ----
export const approvePayout = async (_: FormState, f: FormData) => run(["/payouts"], (z) => z.payouts.approve(id(f)), "finance");
export const rejectPayout = async (_: FormState, f: FormData) => run(["/payouts"], (z) => z.payouts.reject(id(f)), "finance");
export const cancelPayout = async (_: FormState, f: FormData) => run(["/payouts"], (z) => z.payouts.cancel(id(f)), "finance");
export const markPayoutPaid = async (_: FormState, f: FormData) => {
  const ref = String(f.get("bank_ref") ?? "").trim();
  if (ref.length < 6) return { error: "Enter the bank reference (UTR) of the transfer." };
  return run(["/payouts"], (z) => z.payouts.markPaid(id(f), ref), "finance");
};
export const requestPayout = async (_: FormState, f: FormData): Promise<FormState> => {
  const rupees = Number(String(f.get("amount") ?? "").replace(/,/g, ""));
  if (!Number.isFinite(rupees) || rupees <= 0) return { error: "Enter an amount greater than zero." };
  return run(["/payouts"], (z) => z.payouts.create({ vendorId: String(f.get("vendor") ?? ""), amountMinor: Math.round(rupees * 100), purpose: String(f.get("purpose") ?? "").trim() }), "finance");
};
export const addVendor = async (_: FormState, f: FormData) =>
  run(["/payouts"], (z) => z.payouts.createVendor({ name: String(f.get("name") ?? ""), accountNumber: String(f.get("account") ?? "").trim() || undefined, ifsc: String(f.get("ifsc") ?? "").trim().toUpperCase() || undefined, vpa: String(f.get("vpa") ?? "").trim() || undefined }), "finance");

// ---- settings (admin) ----
const rupees = (f: FormData, k: string) => Math.round(Number(String(f.get(k) ?? "0").replace(/,/g, "")) * 100);
const on = (f: FormData, k: string) => f.get(k) === "on";

export const saveCollections = async (_: FormState, f: FormData) =>
  run(["/settings"], (z) => z.settings.update({ dunning: { enabled: on(f, "enabled"), requires_approval: on(f, "requires_approval"), channel: String(f.get("channel") ?? "email") as "email", min_days_overdue: Number(f.get("min_days") ?? 1), repeat_after_days: Number(f.get("repeat_days") ?? 7) } }), "admin");
export const saveApprovals = async (_: FormState, f: FormData) => {
  const [r, p] = [rupees(f, "refund"), rupees(f, "payout")];
  if (!Number.isFinite(r) || !Number.isFinite(p) || r < 0 || p < 0) return { error: "Amounts must be zero or more." };
  return run(["/settings"], (z) => z.settings.update({ refund_approval_threshold_minor: r, payout_approval_threshold_minor: p }), "admin");
};
export const saveGst = async (_: FormState, f: FormData) => {
  const gstin = String(f.get("gstin") ?? "").trim().toUpperCase();
  if (!gstin) return { error: "Enter your GSTIN." };
  return run(["/settings"], (z) => z.settings.update({ gst: { gstin, legal_name: String(f.get("legal_name") ?? "").trim() || undefined } }), "admin");
};
export const saveWebhooks = async (_: FormState, f: FormData) => {
  const events = on(f, "all") ? ["*"] : f.getAll("event").map(String);
  if (events.length === 0) return { error: "Choose at least one event, or all of them." };
  const url = String(f.get("url") ?? "").trim();
  const secret = String(f.get("secret") ?? "").trim();
  return run(["/settings"], (z) => z.settings.update({ webhook_events: events, ...(url ? { outbound_webhook_url: url } : {}), ...(secret ? { outbound_webhook_secret: secret } : {}) }), "admin");
};

// ---- invoices and instalment plans ----
const money = (f: FormData, k: string) => Math.round(Number(String(f.get(k) ?? "").replace(/,/g, "")) * 100);
export const createCustomer = async (_: FormState, f: FormData) =>
  run(["/billing"], (z) => z.customers.create({ name: String(f.get("name") ?? "").trim(), email: String(f.get("email") ?? "").trim() || undefined, phone: String(f.get("phone") ?? "").trim() || undefined, gstin: String(f.get("gstin") ?? "").trim().toUpperCase() || undefined }));
export const createInvoice = async (_: FormState, f: FormData): Promise<FormState> => {
  const amount = money(f, "amount");
  if (!Number.isFinite(amount) || amount <= 0) return { error: "Enter an amount greater than zero." };
  const hsn = String(f.get("hsn") ?? "").trim();
  const base = { number: String(f.get("number") ?? "").trim(), customerId: String(f.get("customer") ?? "") || undefined, dueDate: String(f.get("due") ?? "") || undefined };
  // With an HSN/SAC code the amount is the price before tax and the server adds CGST+SGST or IGST.
  return run(["/billing", "/collections"], (z) =>
    hsn
      ? z.invoices.create({ ...base, placeOfSupply: String(f.get("pos") ?? "").trim() || undefined, lines: [{ description: String(f.get("description") ?? "").trim() || "Services", hsnSac: hsn, quantity: 1, unitPriceMinor: amount, rateBp: Number(f.get("rate") ?? 1800) as 1800 }] })
      : z.invoices.create({ ...base, amountMinor: amount }),
  );
};
export const voidInvoice = async (_: FormState, f: FormData) => run(["/billing", "/collections"], (z) => z.invoices.void(id(f)));
export const createInstalments = async (_: FormState, f: FormData): Promise<FormState> => {
  const total = money(f, "total");
  if (!Number.isFinite(total) || total <= 0) return { error: "Enter the total as an amount greater than zero." };
  return run(["/billing", "/collections"], (z) => z.installments.create({ customerId: String(f.get("customer") ?? ""), name: String(f.get("name") ?? "").trim(), totalMinor: total, instalments: Number(f.get("count") ?? 0), firstDueDate: String(f.get("first_due") ?? ""), numberPrefix: String(f.get("prefix") ?? "").trim() }));
};

export const saveFxRate = async (_: FormState, f: FormData) => {
  const rate = String(f.get("rate") ?? "").trim();
  if (!rate) return { error: "Enter the rate in rupees per US dollar." };
  return run(["/settings", "/collections"], (z) => z.fx.setRate({ currency: "USD", rate }), "admin");
};

export const saveRisk = async (_: FormState, f: FormData) => {
  const num = (k: string) => {
    const v = String(f.get(k) ?? "").trim();
    return v ? Number(v.replace(/,/g, "")) : undefined;
  };
  const maxRupees = num("max_amount");
  if (maxRupees !== undefined && !(maxRupees > 0)) return { error: "The payment ceiling must be more than zero, or empty for none." };
  const blocked = String(f.get("blocked") ?? "").split(/[\s,]+/).map((x) => x.trim().toLowerCase()).filter(Boolean);
  return run(["/settings"], (z) => z.settings.update({ risk: { max_intent_amount_minor: maxRupees === undefined ? undefined : Math.round(maxRupees * 100), max_attempts_per_intent: num("max_attempts"), max_failed_per_customer_per_hour: num("max_failed"), blocked_vpas: blocked } }), "admin");
};

export const createCoupon = async (_: FormState, f: FormData): Promise<FormState> => {
  const kind = String(f.get("kind")) === "flat" ? "flat" : "percent";
  const n = Number(String(f.get("value") ?? "").replace(/,/g, ""));
  if (!Number.isFinite(n) || n <= 0) return { error: "Enter how much the coupon takes off." };
  const uses = String(f.get("max_uses") ?? "").trim();
  return run(["/links"], (z) => z.coupons.create({ code: String(f.get("code") ?? ""), kind, value: Math.round(kind === "percent" ? n * 100 : n * 100), maxUses: uses ? Number(uses) : undefined }));
};
export const deactivateCoupon = async (_: FormState, f: FormData) => run(["/links"], (z) => z.coupons.deactivate(id(f)));
