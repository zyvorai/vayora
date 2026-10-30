import { Zyvor } from "@zyvor/node";

function need(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set. Copy apps/zyvor-console/.env.example to .env.local.`);
  return v;
}

/** Server-side only: the API key never reaches the browser. */
export function api(actor?: string): Zyvor {
  return new Zyvor({ apiKey: need("ZYVOR_API_KEY"), baseUrl: process.env.ZYVOR_API_URL ?? "http://localhost:8080", actor: actor ?? process.env.CONSOLE_USER ?? "console", actorSecret: process.env.ZYVOR_ACTOR_SECRET || undefined });
}

export const authConfig = () => ({ username: process.env.CONSOLE_USERNAME ?? "admin", password: need("CONSOLE_PASSWORD"), secret: need("CONSOLE_SESSION_SECRET") });
