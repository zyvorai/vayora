import { NextResponse } from "next/server";
import { reviewerEnabled, reviewerKey } from "@/lib/reviewer";

// A reviewer opens a document the business uploaded. The API checks the reviewer's key, that the business asked for
// review, and writes the opening to the business's own audit trail; this only carries the bytes across.
export async function GET(_req: Request, ctx: { params: Promise<{ org: string; id: string }> }) {
  if (!reviewerEnabled()) return new NextResponse("Not found", { status: 404 });
  const key = await reviewerKey();
  if (!key) return new NextResponse("Sign in as a reviewer first", { status: 401 });
  const { org, id } = await ctx.params;
  const r = await fetch(`${process.env.ZYVOR_API_URL ?? "http://localhost:8080"}/v1/platform/reviews/${encodeURIComponent(org)}/uploads/${encodeURIComponent(id)}/file`, {
    headers: { authorization: `Bearer ${key}` },
    cache: "no-store",
  });
  if (!r.ok) return new NextResponse(r.status === 404 ? "Not found" : "Could not open the file", { status: r.status === 404 ? 404 : 502 });
  const h = new Headers();
  for (const k of ["content-type", "content-disposition", "x-content-type-options", "cache-control"]) {
    const v = r.headers.get(k);
    if (v) h.set(k, v);
  }
  return new NextResponse(r.body, { headers: h });
}
