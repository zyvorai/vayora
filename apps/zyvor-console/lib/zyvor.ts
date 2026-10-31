import "server-only";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { Zyvor } from "@zyvor/node";
import { currentSession } from "@/lib/auth";
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

/** A person who signed in by email must work with their own key. If it is missing (cookie deleted, secret rotated),
 *  falling back to the deployment key would show another tenant's data, so they are sent to sign in again. */
async function keyForSession(deployment: boolean): Promise<string> {
  if (deployment) return need("ZYVOR_API_KEY");
  const own = await sessionKey();
  if (own) return own;
  if ((await currentSession())?.e) redirect("/login");
  return need("ZYVOR_API_KEY");
}

/** Server-side only: the API key never reaches the browser. A person who signed in by email works as their own
 *  organisation with the key the API issued for their session; the deployment's `ZYVOR_API_KEY` is used only for
 *  the deployment's own sign-in (`deployment: true` is for the sign-in check itself, before any session exists). */
export async function api(actor?: string, opts: { deployment?: boolean } = {}): Promise<Zyvor> {
  const organisation = !opts.deployment && !!(await sessionKey());
  return new Zyvor({
    apiKey: await keyForSession(!!opts.deployment),
    // a person's key that the API no longer accepts (signed out elsewhere, disabled, session over): back to sign-in
    fetch: organisation ? async (url, init) => { const r = await fetch(url, init); if (r.status === 401) redirect("/login"); return r; } : undefined,
    baseUrl: process.env.ZYVOR_API_URL ?? "http://localhost:8080",
    actor: actor ?? process.env.CONSOLE_USER ?? "console",
    actorSecret: process.env.ZYVOR_ACTOR_SECRET || undefined,
  });
}

export const authConfig = () => ({ username: process.env.CONSOLE_USERNAME ?? "admin", password: need("CONSOLE_PASSWORD"), secret: need("CONSOLE_SESSION_SECRET") });

/** The key and address the console talks to the API with, for the few routes that fetch raw files (a QR image). */
export async function apiAccess(): Promise<{ key: string; base: string }> {
  return { key: await keyForSession(false), base: (process.env.ZYVOR_API_URL ?? "http://localhost:8080").replace(/\/$/, "") };
}
