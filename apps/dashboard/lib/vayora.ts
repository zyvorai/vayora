import { Vayora } from "@vayora/node";

function need(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is not set. Copy apps/dashboard/.env.example to .env.local.`);
  return v;
}

/** Server-side only: the API key never reaches the browser. */
export function api(): Vayora {
  return new Vayora({ apiKey: need("VAYORA_API_KEY"), baseUrl: process.env.VAYORA_API_URL ?? "http://localhost:8080" });
}

export const apiUrl = () => process.env.VAYORA_API_URL ?? "http://localhost:8080";
export const apiKey = () => need("VAYORA_API_KEY");
export const authConfig = () => ({ password: need("DASHBOARD_PASSWORD"), secret: need("DASHBOARD_SESSION_SECRET") });
