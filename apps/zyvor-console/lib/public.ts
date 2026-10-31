import "server-only";

export type PublicResult<T> = { ok: true; body: T } | { ok: false; status: number; code?: string; message?: string };

/** A call to the API that needs no key (signup, email sign-in). */
export async function publicPost<T>(path: string, payload: unknown): Promise<PublicResult<T>> {
  try {
    const r = await fetch(`${process.env.ZYVOR_API_URL ?? "http://localhost:8080"}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(payload), cache: "no-store" });
    const body = await r.json().catch(() => ({}));
    if (r.ok) return { ok: true, body: body as T };
    return { ok: false, status: r.status, code: body?.error?.code, message: body?.error?.message };
  } catch {
    return { ok: false, status: 0 };
  }
}

export type EmailLogin = { user: { id: string; username: string; role: "admin" | "finance" | "viewer" | "cashier"; branch_id?: string | null }; mfa_enrolled: boolean; session_key: string };
