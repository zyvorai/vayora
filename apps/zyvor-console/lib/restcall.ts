import { createHmac } from "node:crypto";

/** What a call to the API came back with. A refusal is a value, not an exception, so a page can show it in words. */
export type ApiResult<T> = { ok: true; status: number; body: T } | { ok: false; status: number; code: string; message: string; body: unknown };

export type CallOptions = { base: string; key: string; actor?: string; actorSecret?: string; fetch?: typeof fetch; now?: () => number };

/** The plain sentence a refusal is shown as. The API's own message is used when it sent one. */
export function refusalText(status: number, message: string | undefined): string {
  if (status === 0) return "Could not reach the API. Try again in a moment.";
  if (status === 401) return "You are signed out. Sign in again.";
  if (status === 403) return "Your sign-in is not allowed to do that.";
  if (status === 404) return message || "That was not found.";
  if (status >= 500) return "Zyvor could not do that just now. Try again in a moment.";
  return message || "The request was refused.";
}

/** One JSON call. Headers match the SDK's: the bearer key, and the signed actor so the audit trail names the person. */
export async function callJson<T>(o: CallOptions, method: "GET" | "POST" | "PUT" | "DELETE", path: string, body?: unknown): Promise<ApiResult<T>> {
  const headers: Record<string, string> = { authorization: `Bearer ${o.key}` };
  if (body !== undefined) headers["content-type"] = "application/json";
  if (o.actor) {
    headers["zyvor-actor"] = o.actor;
    if (o.actorSecret) {
      const t = Math.floor((o.now?.() ?? Date.now()) / 1000);
      headers["zyvor-actor-signature"] = `t=${t},v1=${createHmac("sha256", o.actorSecret).update(`${t}.${o.actor}`).digest("hex")}`;
    }
  }
  let res: Response;
  try {
    res = await (o.fetch ?? fetch)(`${o.base.replace(/\/$/, "")}${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), cache: "no-store" });
  } catch {
    return { ok: false, status: 0, code: "unreachable", message: refusalText(0, undefined), body: null };
  }
  const json: unknown = await res.json().catch(() => null);
  if (res.ok) return { ok: true, status: res.status, body: json as T };
  const err = (json as { error?: { code?: string; message?: string } } | null)?.error;
  return { ok: false, status: res.status, code: err?.code ?? "unknown", message: refusalText(res.status, err?.message), body: json };
}

/** Query string from the values that are set, skipping empty ones. */
export function query(p: Record<string, string | number | undefined | null>): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(p)) if (v !== undefined && v !== null && v !== "") q.set(k, String(v));
  const s = q.toString();
  return s ? `?${s}` : "";
}
