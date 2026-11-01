import "server-only";
import { cookies } from "next/headers";
import { notFound } from "next/navigation";
import { openText } from "@/lib/session";

/** Where a reviewer's own key (zp_...) is kept: encrypted and httpOnly, so the browser never holds it. */
export const REVIEWER_COOKIE = "zyvor_reviewer_key";
export const REVIEWER_TTL_SECONDS = 60 * 60 * 8;

/** The reviewer page is for Zyvor's own deployment. A merchant-facing console does not serve it unless this is set. */
export const reviewerEnabled = () => process.env.CONSOLE_REVIEWER === "1";

/** 404 on a deployment that does not turn the page on, so nothing hints that it exists. */
export function requireReviewerSurface(): void {
  if (!reviewerEnabled()) notFound();
}

export async function reviewerKey(): Promise<string | null> {
  const k = await openText(process.env.CONSOLE_SESSION_SECRET ?? "", (await cookies()).get(REVIEWER_COOKIE)?.value);
  return k?.startsWith("zp_") ? k : null;
}

export type PlatformResult<T> = { ok: true; body: T } | { ok: false; status: number; message: string };

/** A call to the platform endpoints with the reviewer's key. */
export async function platform<T>(key: string, method: "GET" | "POST", path: string, payload?: unknown): Promise<PlatformResult<T>> {
  try {
    const r = await fetch(`${process.env.ZYVOR_API_URL ?? "http://localhost:8080"}/v1/platform${path}`, {
      method,
      headers: { authorization: `Bearer ${key}`, ...(payload === undefined ? {} : { "content-type": "application/json" }) },
      body: payload === undefined ? undefined : JSON.stringify(payload),
      cache: "no-store",
    });
    const body = await r.json().catch(() => ({}));
    if (r.ok) return { ok: true, body: body as T };
    return { ok: false, status: r.status, message: body?.error?.message ?? `The API answered ${r.status}.` };
  } catch {
    return { ok: false, status: 0, message: "Could not reach the API." };
  }
}

export type ReviewRow = { organisation_id: string; organisation: string; status: "pending" | "approved" | "rejected" | "revoked"; requested_by: string; requested_at: string; round: number; legal_name: string | null; checks_failed: number; stage: "new" | "awaiting_second"; first_approved_by: string | null; needs_second_because: string[]; assigned_to: string | null; assigned_to_name: string | null; assigned_to_me: boolean; due_at: string; overdue: boolean; sla_hours: number };
export type ReviewDetail = {
  organisation_id: string; organisation: string; status: ReviewRow["status"]; requested_by: string; requested_at: string; round: number; note: string | null; reviewed_by: string | null;
  profile: { legal_name: string; business_type: string; pan: string; gstin: string | null; address: string; status: string; submitted_by: string; reviewed_by: string | null; review_note: string | null; review_reasons: string[] | null } | null;
  documents: { kind: string; reference: string; status: string }[];
  bank_accounts: { holder_name: string; ifsc: string; account_last4: string; verified: boolean }[];
  checks: { kind: string; outcome: "pass" | "fail" | "inconclusive"; detail: string }[];
  stage: "new" | "awaiting_second"; first_approved_by: string | null; needs_second_because: string[]; escalation_note: string | null; assigned_to: string | null; assigned_to_name: string | null; assigned_to_me: boolean; assigned_at: string | null; restore_needs_other: boolean; reviewed_by_name: string | null; due_at: string; overdue: boolean; sla_hours: number;
};

/** How a check outcome is shown: only a pass is green. */
export const outcomeTone = (o: string): "ok" | "bad" | "warn" => (o === "pass" ? "ok" : o === "fail" ? "bad" : "warn");

export type ChangeRow = { id: string; organisation_id: string; organisation: string; kind: "bank_account" | "profile"; summary: string; status: "pending" | "approved" | "rejected"; requested_by: string; requested_at: string; note: string | null; stage: "new" | "awaiting_second"; first_approved_by: string | null; needs_second_because: string[]; due_at: string; overdue: boolean; sla_hours: number };
