import "server-only";
import { cookies } from "next/headers";
import { Zyvor } from "@zyvor/node";
import { KEY_COOKIE, openText } from "@/lib/session";

function need(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set. Copy apps/zyvor-console/.env.example to .env.local.`);
  return v;
}

/** The API key issued for this person's session, or null when signed in with the deployment's own login. */
export async function sessionKey(): Promise<string | null> {
  return openText(process.env.CONSOLE_SESSION_SECRET ?? "", (await cookies()).get(KEY_COOKIE)?.value);
}

/** Server-side only: the API key never reaches the browser. A person who signed in by email works as their own
 *  organisation with the key the API issued for their session; the deployment's `ZYVOR_API_KEY` is the fallback. */
export async function api(actor?: string): Promise<Zyvor> {
  const own = await sessionKey();
  return new Zyvor({ apiKey: own ?? need("ZYVOR_API_KEY"), baseUrl: process.env.ZYVOR_API_URL ?? "http://localhost:8080", actor: actor ?? process.env.CONSOLE_USER ?? "console", actorSecret: process.env.ZYVOR_ACTOR_SECRET || undefined });
}

export const authConfig = () => ({ username: process.env.CONSOLE_USERNAME ?? "admin", password: need("CONSOLE_PASSWORD"), secret: need("CONSOLE_SESSION_SECRET") });

/** The key and address the console talks to the API with, for the few routes that fetch raw files (a QR image). */
export async function apiAccess(): Promise<{ key: string; base: string }> {
  return { key: (await sessionKey()) ?? need("ZYVOR_API_KEY"), base: (process.env.ZYVOR_API_URL ?? "http://localhost:8080").replace(/\/$/, "") };
}
