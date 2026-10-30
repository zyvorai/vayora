// Signed session cookie using Web Crypto, so it works in both proxy.ts and server actions.
export const COOKIE = "zyvor_console_session";
const TTL_SECONDS = 60 * 60 * 12;

const enc = new TextEncoder();
const hex = (b: ArrayBuffer) => Array.from(new Uint8Array(b), (x) => x.toString(16).padStart(2, "0")).join("");

async function mac(secret: string, msg: string): Promise<string> {
  const key = await crypto.subtle.importKey("raw", enc.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return hex(await crypto.subtle.sign("HMAC", key, enc.encode(msg)));
}

function safeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return d === 0;
}

export type Role = "admin" | "finance" | "viewer";
export type Session = { exp: number; /** user id */ i: string; /** username */ u: string; r: Role; /** second factor enrolled */ m: boolean };

const b64 = (s: string) => btoa(String.fromCharCode(...enc.encode(s))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const unb64 = (s: string) => new TextDecoder().decode(Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0)));

/** `payload.signature`; the payload is readable but cannot be changed without the secret. */
export async function createSession(secret: string, who: Omit<Session, "exp">, now = Date.now()): Promise<string> {
  const payload = b64(JSON.stringify({ ...who, exp: Math.floor(now / 1000) + TTL_SECONDS }));
  return `${payload}.${await mac(secret, payload)}`;
}

export async function readSession(secret: string, token: string | undefined, now = Date.now()): Promise<Session | null> {
  if (!token || !secret) return null;
  const [payload, sig] = token.split(".");
  if (!payload || !sig || !safeEqual(sig, await mac(secret, payload))) return null;
  try {
    const s = JSON.parse(unb64(payload)) as Session;
    const ok = typeof s.exp === "number" && s.exp >= Math.floor(now / 1000) && typeof s.u === "string" && ["admin", "finance", "viewer"].includes(s.r);
    return ok ? s : null;
  } catch {
    return null;
  }
}

export const verifySession = async (secret: string, token: string | undefined, now = Date.now()) => (await readSession(secret, token, now)) !== null;

/** viewer < finance < admin. */
export const allows = (have: Role, need: Role) => ["viewer", "finance", "admin"].indexOf(have) >= ["viewer", "finance", "admin"].indexOf(need);

export const SESSION_TTL_SECONDS = TTL_SECONDS;
export const passwordMatches = async (secret: string, a: string, b: string) =>
  // Compare MACs so length differences don't leak through early return.
  safeEqual(await mac(secret, a), await mac(secret, b));
