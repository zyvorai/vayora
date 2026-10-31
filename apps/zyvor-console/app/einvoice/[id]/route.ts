import { NextResponse, type NextRequest } from "next/server";
import { COOKIE, allows, readSession } from "@/lib/session";
import { api } from "@/lib/zyvor";

// A DRAFT e-invoice for a B2B tax invoice, as a JSON download. The proxy already requires a session; this re-checks
// and asks for finance, since the file carries the buyer's GSTIN and address.
export async function GET(req: NextRequest, ctx: { params: Promise<{ id: string }> }) {
  const s = await readSession(process.env.CONSOLE_SESSION_SECRET ?? "", req.cookies.get(COOKIE)?.value);
  if (!s) return new NextResponse("Sign in first", { status: 401 });
  if (!allows(s.r, "finance")) return new NextResponse("Finance or admin only", { status: 403 });
  const { id } = await ctx.params;
  try {
    const doc = await api(s.u).gst.einvoice(id);
    return new NextResponse(JSON.stringify(doc, null, 2), { headers: { "content-type": "application/json; charset=utf-8", "content-disposition": `attachment; filename="einvoice-draft-${id}.json"`, "cache-control": "no-store" } });
  } catch (e) {
    return new NextResponse(e instanceof Error ? e.message : "Could not draft the e-invoice", { status: 422, headers: { "content-type": "text/plain; charset=utf-8" } });
  }
}
