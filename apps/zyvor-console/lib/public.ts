import "server-only";
import { createHmac } from "node:crypto";
import { headers } from "next/headers";
import { visitorIp } from "@/lib/clientip";

export type PublicResult<T> = { ok: true; body: T } | { ok: false; status: number; code?: string; message?: string };

/** Tell the API which visitor this call is for, signed with the secret the API shares with the console, so the
 *  API can rate-limit each visitor separately instead of treating the whole console as one caller. Needs
 *  `ZYVOR_ACTOR_SECRET` and `CONSOLE_TRUST_PROXY_HOPS` (how many proxies sit in front of the console; default 0,
 *  which sends nothing). */
async function vouchedAddress(): Promise<Record<string, string>> {
  const secret = process.env.ZYVOR_ACTOR_SECRET;
  const ip = secret ? visitorIp((await headers()).get("x-forwarded-for"), Number(process.env.CONSOLE_TRUST_PROXY_HOPS ?? 0)) : null;
  if (!secret || !ip) return {};
  const t = Math.floor(Date.now() / 1000);
  return { "zyvor-client-ip": ip, "zyvor-client-ip-signature": `t=${t},v1=${createHmac("sha256", secret).update(`${t}.${ip}`).digest("hex")}` };
}

/** A call to the API that needs no key (signup, email sign-in); `bearer` is for the session key swap. */
export async function publicPost<T>(path: string, payload: unknown, bearer?: string): Promise<PublicResult<T>> {
  try {
    const r = await fetch(`${process.env.ZYVOR_API_URL ?? "http://localhost:8080"}${path}`, { method: "POST", headers: { "content-type": "application/json", ...(bearer ? { authorization: `Bearer ${bearer}` } : {}), ...(await vouchedAddress()) }, body: JSON.stringify(payload), cache: "no-store" });
    const body = await r.json().catch(() => ({}));
    if (r.ok) return { ok: true, body: body as T };
    return { ok: false, status: r.status, code: body?.error?.code, message: body?.error?.message };
  } catch {
    return { ok: false, status: 0 };
  }
}

export type EmailLogin = { user: { id: string; username: string; role: "admin" | "finance" | "viewer" | "cashier"; branch_id?: string | null }; mfa_enrolled: boolean; session_key: string; env: "test" | "live" };
