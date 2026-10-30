import { Zyvor } from "@zyvor/node";

function need(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set. Copy apps/zyvor-console/.env.example to .env.local.`);
  return v;
}

/** Server-side only: the API key never reaches the browser. */
export function api(): Zyvor {
  return new Zyvor({ apiKey: need("ZYVOR_API_KEY"), baseUrl: process.env.ZYVOR_API_URL ?? "http://localhost:8080", actor: process.env.CONSOLE_USER ?? "console" });
}

export const authConfig = () => ({ password: need("CONSOLE_PASSWORD"), secret: need("CONSOLE_SESSION_SECRET") });
