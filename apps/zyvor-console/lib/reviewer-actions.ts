"use server";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { revalidatePath } from "next/cache";
import { sealText } from "@/lib/session";
import { REVIEWER_COOKIE, REVIEWER_TTL_SECONDS, platform, requireReviewerSurface, reviewerKey } from "@/lib/reviewer";

type State = { error?: string; ok?: string } | undefined;

/** Check the key against the API before keeping it; a wrong key is never stored. */
export async function reviewerSignIn(_: State, form: FormData): Promise<State> {
  requireReviewerSurface();
  const key = String(form.get("key") ?? "").trim();
  if (!key.startsWith("zp_")) return { error: "That is not a reviewer key. Reviewer keys start with zp_." };
  const secret = process.env.CONSOLE_SESSION_SECRET ?? "";
  if (!secret) return { error: "This console has no session secret set." };
  const r = await platform(key, "GET", "/reviews?status=pending");
  if (!r.ok) return { error: r.status === 401 ? "The API does not accept that key. It may be disabled." : r.message };
  (await cookies()).set(REVIEWER_COOKIE, await sealText(secret, key), { httpOnly: true, sameSite: "strict", secure: process.env.NODE_ENV === "production", path: "/reviewer", maxAge: REVIEWER_TTL_SECONDS });
  redirect("/reviewer");
}

export async function reviewerSignOut() {
  requireReviewerSurface();
  (await cookies()).delete({ name: REVIEWER_COOKIE, path: "/reviewer" });
  redirect("/reviewer");
}

/** Approve, or reject with the note the merchant will be shown. */
export async function decideReview(_: State, form: FormData): Promise<State> {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) redirect("/reviewer");
  const org = String(form.get("org") ?? "");
  const decision = String(form.get("decision") ?? "");
  const note = String(form.get("note") ?? "").trim();
  if (decision !== "approve" && decision !== "reject") return { error: "Choose approve or reject." };
  if (decision === "reject" && !note) return { error: "Say what the merchant should fix. They are shown this." };
  const r = await platform(key, "POST", `/reviews/${encodeURIComponent(org)}/decision`, { decision, note: note || undefined });
  if (!r.ok) {
    if (r.status === 401) redirect("/reviewer");
    return { error: r.message };
  }
  revalidatePath("/reviewer");
  // the first of two approvals leaves the review pending: stay on it so the next reviewer is not a surprise
  if ((r.body as { stage?: string } | undefined)?.stage === "awaiting_second") redirect(`/reviewer?status=pending&done=first_approval`);
  redirect(`/reviewer?status=pending&done=${decision}`);
}

/** Take live mode away from an approved business, or give it back. Taking it away needs the reason the merchant will see. */
export async function changeLiveAccess(_: State, form: FormData): Promise<State> {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) redirect("/reviewer");
  const org = String(form.get("org") ?? "");
  const action = String(form.get("action") ?? "");
  const note = String(form.get("note") ?? "").trim();
  if (action !== "revoke" && action !== "restore") return { error: "Choose revoke or restore." };
  if (action === "revoke" && !note) return { error: "Say why. The merchant is shown this." };
  if (action === "revoke" && form.get("confirm") !== "yes") return { error: "Tick the box to confirm: new payments and payouts stop at once." };
  const r = await platform(key, "POST", `/reviews/${encodeURIComponent(org)}/${action}`, { note: note || undefined });
  if (!r.ok) {
    if (r.status === 401) redirect("/reviewer");
    return { error: r.message };
  }
  revalidatePath("/reviewer");
  redirect(`/reviewer?status=${action === "revoke" ? "revoked" : "approved"}&done=${action}`);
}

/** Approve or reject a change a live business made (a new bank account). Rejecting needs the reason the merchant sees. */
export async function decideChange(_: State, form: FormData): Promise<State> {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) redirect("/reviewer");
  const id = String(form.get("id") ?? "");
  const decision = String(form.get("decision") ?? "");
  const note = String(form.get("note") ?? "").trim();
  if (decision !== "approve" && decision !== "reject") return { error: "Choose approve or reject." };
  if (decision === "reject" && !note) return { error: "Say why. The merchant is shown this, and the account is removed." };
  const r = await platform<{ stage?: string }>(key, "POST", `/changes/${encodeURIComponent(id)}/decision`, { decision, note: note || undefined });
  if (!r.ok) {
    if (r.status === 401) redirect("/reviewer");
    return { error: r.message };
  }
  revalidatePath("/reviewer");
  redirect(`/reviewer?status=pending&done=${r.body.stage === "awaiting_second" ? "change_first_approval" : `change_${decision}`}`);
}

/** Ask for a second, different reviewer. The note is read by whoever gives the second approval. */
export async function escalateReview(_: State, form: FormData): Promise<State> {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) redirect("/reviewer");
  const org = String(form.get("org") ?? "");
  const note = String(form.get("note") ?? "").trim();
  if (!note) return { error: "Say why. The second reviewer reads this." };
  const r = await platform(key, "POST", `/reviews/${encodeURIComponent(org)}/escalate`, { note });
  if (!r.ok) {
    if (r.status === 401) redirect("/reviewer");
    return { error: r.message };
  }
  revalidatePath(`/reviewer/review/${org}`);
  return { ok: "Done. A different reviewer must now approve it too." };
}

/** Take a review so only you decide it, hand it back, or take it from whoever holds it (on purpose, and audited). */
export async function assignReview(_: State, form: FormData): Promise<State> {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) redirect("/reviewer");
  const change = String(form.get("change") ?? "");
  const org = String(form.get("org") ?? "");
  const how = String(form.get("how") ?? "");
  if (!["claim", "release", "take_over"].includes(how)) return { error: "Choose what to do with this review." };
  const body = how === "release" ? {} : { to: "me", take_over: how === "take_over" };
  const r = await platform(key, "POST", change ? `/changes/${encodeURIComponent(change)}/assign` : `/reviews/${encodeURIComponent(org)}/assign`, body);
  if (!r.ok) {
    if (r.status === 401) redirect("/reviewer");
    return { error: r.message };
  }
  if (org) revalidatePath(`/reviewer/review/${org}`);
  revalidatePath("/reviewer");
  return { ok: how === "release" ? "Handed back to the queue." : `This ${change ? "change" : "review"} is yours. Nobody else can decide it until you hand it back.` };
}

/** Ask for a second, different reviewer on a pending change no rule flagged. The note is read by whoever gives the second approval. */
export async function escalateChange(_: State, form: FormData): Promise<State> {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) redirect("/reviewer");
  const id = String(form.get("id") ?? "");
  const note = String(form.get("note") ?? "").trim();
  if (!note) return { error: "Say why. The second reviewer reads this." };
  const r = await platform(key, "POST", `/changes/${encodeURIComponent(id)}/escalate`, { note });
  if (!r.ok) {
    if (r.status === 401) redirect("/reviewer");
    return { error: r.message };
  }
  revalidatePath("/reviewer");
  return { ok: "Done. A different reviewer must now approve it too." };
}

const DAY_KEYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/** Replace the signed-in reviewer's weekly hours: one shift per ticked day. */
export async function saveRota(_: State, form: FormData): Promise<State> {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) redirect("/reviewer");
  const shifts = DAY_KEYS.filter((d) => form.get(`${d}_on`) === "on").map((d) => ({ weekday: d, start: String(form.get(`${d}_start`) ?? ""), end: String(form.get(`${d}_end`) ?? "") }));
  const r = await platform(key, "PUT", "/rota/me", { shifts });
  if (!r.ok) {
    if (r.status === 401) redirect("/reviewer");
    return { error: r.message };
  }
  revalidatePath("/reviewer/rota");
  revalidatePath("/reviewer");
  return { ok: shifts.length ? "Your hours are saved." : "Your hours are cleared; you are not on the rota." };
}
