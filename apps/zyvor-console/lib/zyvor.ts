import "server-only";
import { cookies } from "next/headers";
import { Zyvor } from "@zyvor/node";
import { KEY_COOKIE, openText } from "@/lib/session";

function need(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set. Copy apps/zyvor-console/.env.example to .env.local.`);
  return v;
}

/** Server-side only: the API key never reaches the browser. A person who signed in by email works as their own
 *  organisation with the key the API issued for their session; the deployment's `ZYVOR_API_KEY` is the fallback. */
export async function api(actor?: string): Promise<Zyvor> {
  const own = await openText(process.env.CONSOLE_SESSION_SECRET ?? "", (await cookies()).get(KEY_COOKIE)?.value);
  return new Zyvor({ apiKey: own ?? need("ZYVOR_API_KEY"), baseUrl: process.env.ZYVOR_API_URL ?? "http://localhost:8080", actor: actor ?? process.env.CONSOLE_USER ?? "console", actorSecret: process.env.ZYVOR_ACTOR_SECRET || undefined });
}

export const authConfig = () => ({ username: process.env.CONSOLE_USERNAME ?? "admin", password: need("CONSOLE_PASSWORD"), secret: need("CONSOLE_SESSION_SECRET") });
