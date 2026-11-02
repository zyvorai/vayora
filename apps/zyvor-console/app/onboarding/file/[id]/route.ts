import { NextResponse, type NextRequest } from "next/server";
import { ZyvorError } from "@zyvor/node";
import { COOKIE, readSession } from "@/lib/session";
import { api } from "@/lib/zyvor";

// The proxy already requires a session; this re-checks so the route is safe if the matcher changes.
export async function GET(req: NextRequest, ctx: { params: Promise<{ id: string }> }) {
  const s = await readSession(process.env.CONSOLE_SESSION_SECRET ?? "", req.cookies.get(COOKIE)?.value);
  if (!s) return new NextResponse("Sign in first", { status: 401 });
  const { id } = await ctx.params;
  const z = await api(s.u);
  try {
    const list = await z.onboarding.uploads.list();
    const u = list.data.find((x) => x.id === id);
    if (!u) return new NextResponse("Not found", { status: 404 });
    const bytes = await z.onboarding.uploads.download(id);
    return new NextResponse(bytes as unknown as BodyInit, {
      headers: {
        "content-type": u.content_type,
        "content-disposition": `attachment; filename="${u.filename.replace(/[^\w. -]/g, "_")}"`,
        "x-content-type-options": "nosniff",
        "cache-control": "no-store",
      },
    });
  } catch (e) {
    if (e instanceof ZyvorError) return new NextResponse(e.message, { status: e.status === 404 ? 404 : 502 });
    throw e;
  }
}
