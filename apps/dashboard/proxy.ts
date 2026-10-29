import { NextResponse, type NextRequest } from "next/server";
import { COOKIE, verifySession } from "@/lib/session";

// Everything except the login page and static assets requires a valid signed session.
export async function proxy(req: NextRequest) {
  const secret = process.env.DASHBOARD_SESSION_SECRET ?? "";
  if (await verifySession(secret, req.cookies.get(COOKIE)?.value)) return NextResponse.next();
  const url = req.nextUrl.clone();
  url.pathname = "/login";
  url.search = req.nextUrl.pathname === "/" ? "" : `?next=${encodeURIComponent(req.nextUrl.pathname + req.nextUrl.search)}`;
  return NextResponse.redirect(url);
}

export const config = { matcher: ["/((?!login|_next/|favicon.svg).*)"] };
