"use server";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { revalidatePath } from "next/cache";
import { ZyvorError, type Zyvor } from "@zyvor/node";
import { currentSession } from "@/lib/auth";
import { parseBulk } from "@/lib/bulk";
import { publicPost, type EmailLogin } from "@/lib/public";
import { COOKIE, KEY_COOKIE, SESSION_TTL_SECONDS, sealText, allows, createSession, passwordMatches, type Role, type Session } from "@/lib/session";
import { api, authConfig, sessionKey } from "@/lib/zyvor";

export type FormState = { ok?: string; error?: string; otp?: boolean; secret?: string; uri?: string; codes?: string[] } | undefined;

async function startWithKey(who: Omit<Session, "exp">, key: string) {
  await start(who);
  (await cookies()).set(KEY_COOKIE, await sealText(authConfig().secret, key), {
    httpOnly: true,
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
    path: "/",
    maxAge: SESSION_TTL_SECONDS,
  });
}

/** Sign in by email as a member of the organisation that signed up with it. */
async function emailSignIn(email: string, password: string, otp: string): Promise<{ who: Omit<Session, "exp">; key: string } | NonNullable<FormState>> {
  const r = await publicPost<EmailLogin>("/v1/console/login", { email, password, otp: otp || undefined });
  if (r.ok) return { who: { i: r.body.user.id, u: r.body.user.username, r: r.body.user.role, m: r.body.mfa_enrolled, e: r.body.env, ...(r.body.user.branch_id ? { b: r.body.user.branch_id } : {}) }, key: r.body.session_key };
  if (r.code === "otp_required") return { otp: true };
  if (r.status === 423) return { error: "Too many failed attempts. Try again in 15 minutes." };
  if (r.status !== 401) return { error: "Sign-in is unavailable right now. Try again in a moment." };
  await new Promise((x) => setTimeout(x, 500)); // slow down guessing
  return { error: "Incorrect email, password or code." };
}

function safeNext(form: FormData): string {
  const next = String(form.get("next") ?? "/");
  // Only same-site relative paths; never an open redirect.
  return next.startsWith("/") && !next.startsWith("//") ? next : "/";
}

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
  if (u.includes("@")) {
    const r = await emailSignIn(u, p, otp);
    if ("who" in r) {
      await startWithKey(r.who, r.key);
      redirect(safeNext(form));
    }
    // not an organisation's email: fall through to the deployment's own users
    if (r.otp || (r.error && !r.error.startsWith("Incorrect"))) return r;
  }
  try {
    const r = await (await api()).consoleUsers.authenticate({ username: u, password: p, otp: otp || undefined });
    signedIn = { i: r.user.id, u: r.user.username, r: r.user.role, m: r.mfa_enrolled, ...(r.user.branch_id ? { b: r.user.branch_id } : {}) };
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
        noUsers = (await (await api()).consoleUsers.list()).data.length === 0;
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
  redirect(safeNext(form));
}

/** Move to the organisation's other environment. The API swaps the session key; live needs approved onboarding. */
export async function switchEnv(_: FormState, form: FormData): Promise<FormState> {
  const s = await currentSession();
  const key = await sessionKey();
  const env = form.get("env") === "live" ? "live" : "test";
  if (!s || !key) return { error: "Switching is available when you sign in with your email." };
  if (s.e === env) return undefined;
  const r = await publicPost<EmailLogin>("/v1/console/switch_env", { env }, key);
  if (!r.ok) {
    if (r.code === "onboarding_incomplete") return { error: "Live mode opens once your business details are approved. Finish onboarding first." };
    return { error: "Could not switch right now. Try again in a moment." };
  }
  const { exp: _exp, ...who } = s;
  await startWithKey({ ...who, e: r.body.env }, r.body.session_key);
  revalidatePath("/", "layout");
  return undefined;
}

export async function signup(_: FormState, form: FormData): Promise<FormState> {
  const email = String(form.get("email") ?? "").trim().toLowerCase();
  const password = String(form.get("password") ?? "");
  const r = await publicPost("/v1/signup", { organisation_name: String(form.get("organisation") ?? ""), email, password });
  if (!r.ok) {
    if (r.status === 409) return { error: "An account already exists for that email. Sign in instead." };
    if (r.status === 422) return { error: r.message ? `${r.message[0].toUpperCase()}${r.message.slice(1)}.` : "Check the details and try again." };
    if (r.status === 429) return { error: "Too many attempts. Try again in a minute." };
    return { error: "Signup is unavailable right now. Try again in a moment." };
  }
  const si = await emailSignIn(email, password, "");
  if (!("who" in si)) redirect("/login");
  await startWithKey(si.who, si.key);
  redirect("/onboarding");
}

export async function logout() {
  (await cookies()).delete(COOKIE);
  (await cookies()).delete(KEY_COOKIE);
  redirect("/login");
}

const DENIED = { error: "Your role does not allow that." };

/** Run an API call from a form as the signed-in person. A refusal (state changed, not allowed) is shown, not thrown. */
async function run(paths: string[], fn: (z: Zyvor, s: Session) => Promise<unknown>, need: Role = "finance"): Promise<FormState> {
  const s = await currentSession();
  if (!s || !allows(s.r, need)) return DENIED;
  try {
    await fn(await api(s.u), s);
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
/** Refund all or part of a payment. The key comes from the form, so a double click is one refund, not two. */
export const refundPayment = async (_: FormState, f: FormData) => {
  const raw = String(f.get("amount") ?? "").replace(/,/g, "").trim();
  const rupees = raw === "" ? undefined : Number(raw);
  if (rupees !== undefined && (!Number.isFinite(rupees) || rupees <= 0)) return { error: "Enter an amount greater than zero, or leave it empty to refund what is left." };
  const key = String(f.get("key") ?? "");
  const intent = id(f);
  const r = await run([`/payments/${intent}`, "/refunds", "/"], (z) => z.intents.refund(intent, { amountMinor: rupees === undefined ? undefined : Math.round(rupees * 100), reason: String(f.get("reason") ?? "").trim() || undefined, idempotencyKey: key || undefined }));
  return r?.ok ? { ok: "Refund requested. If it is above your approval limit it waits for a second person on the Refunds page." } : r;
};
export const sendTestEvent = async (_: FormState) => run(["/webhooks", "/developers"], (z) => z.webhooks.sendTest(), "admin");
export const assignCase = async (_: FormState, f: FormData) => run(["/reconciliation", "/"], (z, s) => z.cases.assign(id(f), String(f.get("assignee") ?? "").trim() || s.u));
export const resolveCase = async (_: FormState, f: FormData) => {
  const note = String(f.get("resolution") ?? "").trim();
  if (!note) return { error: "Say how it was resolved." };
  return run(["/reconciliation", "/"], (z) => z.cases.resolve(id(f), note));
};
export const replayEvent = async (_: FormState, f: FormData) => run(["/webhooks"], (z) => z.webhooks.replay(id(f)), "admin");
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
  const money = (k: string) => {
    const v = String(f.get(k) ?? "").trim();
    return v ? Math.round(Number(v.replace(/,/g, "")) * 100) : undefined;
  };
  const open = f.get("open") === "on";
  const maxUses = String(f.get("max_uses") ?? "").trim();
  const description = String(f.get("description") ?? "").trim() || undefined;
  const counterId = String(f.get("counter_id") ?? "") || undefined;
  let path = "";
  if (open) {
    const [min, max] = [money("min"), money("max")];
    if ([min, max].some((v) => v !== undefined && (!Number.isFinite(v) || v <= 0))) return { error: "Enter the smallest and largest amounts in rupees, or leave them empty." };
    if (min !== undefined && max !== undefined && min > max) return { error: "The smallest amount cannot be more than the largest." };
    const r = await run(["/links"], async (z) => {
      path = (await z.paymentLinks.create({ description, openAmount: true, minAmountMinor: min, maxAmountMinor: max, counterId })).path;
    });
    return r?.error ? r : { ok: path };
  }
  const amount = money("amount");
  if (amount === undefined || !Number.isFinite(amount) || amount <= 0) return { error: "Enter an amount greater than zero." };
  const r = await run(["/links"], async (z) => {
    path = (await z.paymentLinks.create({
      description,
      amountMinor: amount,
      maxUses: maxUses ? Number(maxUses) : undefined,
      convenienceFeeBp: String(f.get("fee") ?? "").trim() ? Math.round(Number(String(f.get("fee")).replace(/,/g, "")) * 100) : undefined,
      counterId,
    })).path;
  });
  return r?.error ? r : { ok: path };
};
export const cancelLink = async (_: FormState, f: FormData) => run(["/links"], (z) => z.paymentLinks.cancel(id(f)));

// ---- people and security (admin) ----
export const createUser = async (_: FormState, f: FormData) =>
  run(["/users"], (z) => z.consoleUsers.create({ username: String(f.get("username") ?? ""), password: String(f.get("password") ?? ""), role: String(f.get("role") ?? "viewer") as Role, branchId: String(f.get("branch_id") ?? "") || undefined }), "admin");
export const disableUser = async (_: FormState, f: FormData) => run(["/users"], (z, s) => (id(f) === s.i ? Promise.reject(new ZyvorError(422, "self", "You cannot disable yourself.")) : z.consoleUsers.disable(id(f))), "admin");
export const enableUser = async (_: FormState, f: FormData) => run(["/users"], (z) => z.consoleUsers.enable(id(f)), "admin");
export const resetUserMfa = async (_: FormState, f: FormData) => run(["/users"], (z) => z.consoleUsers.mfaReset(id(f)), "admin");

// ---- your own second factor (any signed-in named user) ----
export async function beginMfa(): Promise<FormState> {
  const s = await currentSession();
  if (!s || s.i === "bootstrap") return { error: "Create a named user first; the deployment admin cannot enrol a device." };
  try {
    const r = await (await api(s.u)).consoleUsers.mfaBegin(s.i);
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
    const r = await (await api(s.u)).consoleUsers.mfaConfirm(s.i, String(f.get("otp") ?? "").trim());
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
  return run(["/payouts"], (z) => z.payouts.create({ vendorId: String(f.get("vendor") ?? ""), amountMinor: Math.round(rupees * 100), purpose: String(f.get("purpose") ?? "").trim(), fromBalance: f.get("from_balance") === "on" }), "finance");
};

export const bulkPayouts = async (_: FormState, f: FormData): Promise<FormState> => {
  const fromBalance = f.get("from_balance") === "on";
  let made = "";
  const r = await run(["/payouts"], async (z) => {
    const vendors = (await z.payouts.vendors()).data;
    const parsed = parseBulk(String(f.get("csv") ?? ""), vendors);
    if ("error" in parsed) throw new ZyvorError(422, "invalid_bulk", parsed.error);
    const out = await z.payouts.createBulk(parsed.items.map((i) => ({ ...i, fromBalance })));
    const fresh = out.count - out.already_existed;
    made = fresh === 0 ? `Nothing new: all ${out.count} already existed.` : `${fresh} created${out.already_existed ? `, ${out.already_existed} already existed` : ""}. Each still needs a second person's approval.`;
  }, "finance");
  return r?.error ? r : { ok: made };
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

const text = (f: FormData, k: string) => String(f.get(k) ?? "").trim();
export const saveProfile = async (_: FormState, f: FormData) =>
  run(["/onboarding"], (z) => z.onboarding.saveProfile({ legalName: text(f, "legal_name"), businessType: text(f, "business_type"), pan: text(f, "pan"), gstin: text(f, "gstin") || undefined, address: text(f, "address") }), "admin");
export const addOnboardingDoc = async (_: FormState, f: FormData) => run(["/onboarding"], (z) => z.onboarding.addDocument({ kind: text(f, "kind"), reference: text(f, "reference") }), "admin");
export const addBankAccount = async (_: FormState, f: FormData) =>
  run(["/onboarding"], (z) => z.onboarding.addBankAccount({ holderName: text(f, "holder_name"), ifsc: text(f, "ifsc"), accountNumber: text(f, "account_number").replace(/\s/g, "") }), "admin");
export const verifyBankAccount = async (_: FormState, f: FormData) => run(["/onboarding"], (z) => z.onboarding.verifyBankAccount(id(f)), "admin");
export const submitOnboarding = async (_: FormState) => run(["/onboarding"], (z) => z.onboarding.submit(), "admin");
export const reviewOnboarding = async (_: FormState, f: FormData) =>
  run(["/onboarding"], (z) => z.onboarding.review({ decision: text(f, "decision") === "approve" ? "approve" : "reject", note: text(f, "note") || undefined }), "admin");

export const createBranch = async (_: FormState, f: FormData) => run(["/stores"], (z) => z.branches.create(text(f, "name")), "admin");
export const createCounter = async (_: FormState, f: FormData) => run(["/stores"], (z) => z.counters.create({ branchId: text(f, "branch_id"), name: text(f, "name") }), "admin");
export const deactivateCounter = async (_: FormState, f: FormData) => run(["/stores"], (z) => z.counters.deactivate(id(f)), "admin");
export const addCash = async (_: FormState, f: FormData): Promise<FormState> => {
  const amt = rupees(f, "amount");
  if (!Number.isFinite(amt) || amt <= 0) return { error: "Enter the amount in rupees." };
  const kind = text(f, "kind");
  return run(["/closing"], (z, s) => z.cash.add({ branchId: s.b ?? text(f, "branch_id"), kind: kind === "refund" ? "refund" : kind === "float" ? "float" : "sale", amountMinor: amt, note: text(f, "note") || undefined }), "cashier");
};
export const createClosing = async (_: FormState, f: FormData): Promise<FormState> => {
  const amt = rupees(f, "counted");
  if (!Number.isFinite(amt) || amt < 0) return { error: "Enter the cash you counted, in rupees." };
  return run(["/closing"], (z, s) => z.closings.create({ branchId: s.b ?? text(f, "branch_id"), day: text(f, "day"), countedCashMinor: amt, note: text(f, "note") || undefined }), "cashier");
};
export const signOffClosing = async (_: FormState, f: FormData) => run(["/closing"], (z) => z.closings.signOff(id(f)), "admin");

/** The secret is returned once, in `ok`, and never stored. */
export const createKey = async (_: FormState, f: FormData): Promise<FormState> => {
  const preset = text(f, "preset");
  const scopes = preset === "read" ? ["payments:read"] : ["payments:read", "payments:write"];
  const branchId = text(f, "branch_id") || undefined;
  const name = text(f, "name");
  if (!name) return { error: "Give the key a name so you can tell it apart later." };
  let secret = "";
  const r = await run(["/keys"], async (z) => {
    secret = (await z.keys.create({ name, scopes, branchId })).key;
  }, "admin");
  return r?.error ? r : { ok: secret };
};
export const revokeKey = async (_: FormState, f: FormData) => run(["/keys"], (z) => z.keys.revoke(id(f)), "admin");
