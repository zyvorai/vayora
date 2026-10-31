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

export type Role = "admin" | "finance" | "viewer" | "cashier";
export type Session = { exp: number; /** user id */ i: string; /** username */ u: string; r: Role; /** second factor enrolled */ m: boolean; /** branch a cashier is tied to */ b?: string };

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
    const ok = typeof s.exp === "number" && s.exp >= Math.floor(now / 1000) && typeof s.u === "string" && ["admin", "finance", "viewer", "cashier"].includes(s.r);
    return ok ? s : null;
  } catch {
    return null;
  }
}

export const verifySession = async (secret: string, token: string | undefined, now = Date.now()) => (await readSession(secret, token, now)) !== null;

const ORDER: Role[] = ["cashier", "viewer", "finance", "admin"];
/** cashier < viewer < finance < admin. A cashier reaches only the till (see `tillOnly`). */
export const allows = (have: Role, need: Role) => ORDER.indexOf(have) >= ORDER.indexOf(need);
/** Pages a cashier may open: the till and their own account. */
export const tillOnly = (path: string) => path === "/closing" || path.startsWith("/closing/") || path === "/account" || path.startsWith("/account/");

export const SESSION_TTL_SECONDS = TTL_SECONDS;
export const passwordMatches = async (secret: string, a: string, b: string) =>
  // Compare MACs so length differences don't leak through early return.
  safeEqual(await mac(secret, a), await mac(secret, b));

/** Holds the per-person API key the API issued at sign-in. Encrypted, so the browser never sees it. */
export const KEY_COOKIE = "zyvor_console_key";

async function aesKey(secret: string, usage: KeyUsage): Promise<CryptoKey> {
  const raw = await crypto.subtle.digest("SHA-256", enc.encode(`key-cookie:${secret}`));
  return crypto.subtle.importKey("raw", raw, "AES-GCM", false, [usage]);
}

/** AES-GCM with a random nonce, base64url of `nonce || ciphertext`. */
export async function sealText(secret: string, text: string): Promise<string> {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, await aesKey(secret, "encrypt"), enc.encode(text)));
  const all = new Uint8Array(iv.length + ct.length);
  all.set(iv);
  all.set(ct, iv.length);
  return btoa(String.fromCharCode(...all)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export async function openText(secret: string, sealed: string | undefined): Promise<string | null> {
  if (!sealed || !secret) return null;
  try {
    const all = Uint8Array.from(atob(sealed.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
    const pt = await crypto.subtle.decrypt({ name: "AES-GCM", iv: all.slice(0, 12) }, await aesKey(secret, "decrypt"), all.slice(12));
    return new TextDecoder().decode(pt);
  } catch {
    return null;
  }
}
