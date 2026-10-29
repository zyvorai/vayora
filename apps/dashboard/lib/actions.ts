"use server";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { revalidatePath } from "next/cache";
import { VayoraError } from "@vayora/node";
import { COOKIE, SESSION_TTL_SECONDS, createSession, passwordMatches } from "@/lib/session";
import { parseRupees, inr } from "@/lib/format";
import { api, authConfig } from "@/lib/vayora";

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

export async function refundPayment(_: FormState, form: FormData): Promise<FormState> {
  const paymentId = String(form.get("payment_id") ?? "");
  const key = String(form.get("idempotency_key") ?? "");
  const raw = String(form.get("amount") ?? "").trim();
  let amountPaise: number | undefined;
  if (raw) {
    const parsed = parseRupees(raw);
    if (parsed === null) return { error: "Enter an amount like 250 or 250.50." };
    amountPaise = parsed;
  }
  try {
    const r = await api().refunds.create({ paymentId, amountPaise, idempotencyKey: key });
    revalidatePath(`/payments/${paymentId}`);
    revalidatePath("/refunds");
    revalidatePath("/");
    return { ok: `Refunded ${inr(r.amount_paise)}.` };
  } catch (e) {
    if (e instanceof VayoraError) return { error: e.message };
    throw e;
  }
}

export async function retryEvent(form: FormData) {
  await api().events.retry(String(form.get("id") ?? "")).catch((e) => {
    if (!(e instanceof VayoraError)) throw e; // already delivered / gone: just refresh
  });
  revalidatePath("/webhooks");
}
