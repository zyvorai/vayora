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
