import { NextResponse, type NextRequest } from "next/server";
import { COOKIE, readSession } from "@/lib/session";
import { apiAccess } from "@/lib/zyvor";

// The QR code of a payment link, fetched with the signed-in person's key so the browser never holds it.
export async function GET(req: NextRequest, ctx: { params: Promise<{ id: string }> }) {
  const s = await readSession(process.env.CONSOLE_SESSION_SECRET ?? "", req.cookies.get(COOKIE)?.value);
  if (!s) return new NextResponse("Sign in first", { status: 401 });
  const { id } = await ctx.params;
  const { key, base } = await apiAccess();
  const r = await fetch(`${base}/v1/payment_links/${encodeURIComponent(id)}/qr.svg`, { headers: { authorization: `Bearer ${key}` }, cache: "no-store" });
  if (!r.ok) return new NextResponse("No QR code for this link", { status: r.status === 404 ? 404 : 502 });
  return new NextResponse(await r.text(), { headers: { "content-type": "image/svg+xml", "cache-control": "private, max-age=300", "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'" } });
}
