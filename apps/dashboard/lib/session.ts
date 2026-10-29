// Signed session cookie using Web Crypto, so it works in both proxy.ts and server actions.
export const COOKIE = "vayora_session";
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

export async function createSession(secret: string, now = Date.now()): Promise<string> {
  const exp = Math.floor(now / 1000) + TTL_SECONDS;
  return `${exp}.${await mac(secret, String(exp))}`;
}

export async function verifySession(secret: string, token: string | undefined, now = Date.now()): Promise<boolean> {
  if (!token || !secret) return false;
  const [exp, sig] = token.split(".");
  if (!exp || !sig || !/^\d+$/.test(exp) || Number(exp) < Math.floor(now / 1000)) return false;
  return safeEqual(sig, await mac(secret, exp));
}

export const SESSION_TTL_SECONDS = TTL_SECONDS;
export const passwordMatches = async (secret: string, a: string, b: string) =>
  // Compare MACs so length differences don't leak through early return.
  safeEqual(await mac(secret, a), await mac(secret, b));
