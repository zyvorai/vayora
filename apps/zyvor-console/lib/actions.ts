"use server";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { revalidatePath } from "next/cache";
import { ZyvorError } from "@zyvor/node";
import { COOKIE, SESSION_TTL_SECONDS, createSession, passwordMatches } from "@/lib/session";
import { api, authConfig } from "@/lib/zyvor";

export type FormState = { ok?: string; error?: string } | undefined;

export async function login(_: FormState, form: FormData): Promise<FormState> {
  const { password, secret } = authConfig();
  const ok = await passwordMatches(secret, String(form.get("password") ?? ""), password);
  if (!ok) {
    await new Promise((r) => setTimeout(r, 500)); // slow down guessing
    return { error: "Incorrect password." };
  }
  (await cookies()).set(COOKIE, await createSession(secret), {
    httpOnly: true,
    sameSite: "lax",
    secure: process.env.NODE_ENV === "production",
    path: "/",
    maxAge: SESSION_TTL_SECONDS,
  });
  const next = String(form.get("next") ?? "/");
  // Only same-site relative paths; never an open redirect.
  redirect(next.startsWith("/") && !next.startsWith("//") ? next : "/");
}

export async function logout() {
  (await cookies()).delete(COOKIE);
  redirect("/login");
}

/** Run an API call from a form; an API refusal (state changed, not allowed) is shown, not thrown. */
async function run(paths: string[], fn: () => Promise<unknown>): Promise<FormState> {
  try {
    await fn();
  } catch (e) {
    if (e instanceof ZyvorError) return { error: e.message };
    throw e;
  }
  for (const p of paths) revalidatePath(p);
  return { ok: "Done." };
}
const id = (f: FormData) => String(f.get("id") ?? "");

export const approveRefund = async (_: FormState, f: FormData) => run(["/refunds", "/"], () => api().refunds.approve(id(f)));
export const rejectRefund = async (_: FormState, f: FormData) => run(["/refunds", "/"], () => api().refunds.reject(id(f)));
export const retryRefund = async (_: FormState, f: FormData) => run(["/refunds"], () => api().refunds.retry(id(f)));
export const assignCase = async (_: FormState, f: FormData) => run(["/reconciliation", "/"], () => api().cases.assign(id(f), String(f.get("assignee") ?? "").trim() || (process.env.CONSOLE_USER ?? "console")));
export const resolveCase = async (_: FormState, f: FormData) => {
  const note = String(f.get("resolution") ?? "").trim();
  if (!note) return { error: "Say how it was resolved." };
  return run(["/reconciliation", "/"], () => api().cases.resolve(id(f), note));
};
export const replayEvent = async (_: FormState, f: FormData) => run(["/webhooks"], () => api().webhooks.replay(id(f)));
export const runBilling = async (_: FormState): Promise<FormState> => {
  try {
    const r = await api().billing.run();
    revalidatePath("/billing");
    return { ok: `${r.invoices_created} invoice(s) created.` };
  } catch (e) {
    if (e instanceof ZyvorError) return { error: e.message };
    throw e;
  }
};
export const remind = async (_: FormState, f: FormData) => run(["/collections"], () => api().collections.remind(id(f), { channel: String(f.get("channel") ?? "whatsapp") as "whatsapp" }));
export const completeMilestone = async (_: FormState, f: FormData) => run(["/billing"], () => api().milestones.complete(id(f)));
export const resolveDispute = async (_: FormState, f: FormData) => run(["/disputes", "/"], () => api().disputes.resolve(id(f), String(f.get("outcome")) === "won" ? "won" : "lost"));
