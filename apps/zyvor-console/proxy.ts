import { NextResponse, type NextRequest } from "next/server";
import { COOKIE, readSession, tillOnly } from "@/lib/session";

// Everything except the login page and static assets requires a valid signed session. With
// CONSOLE_REQUIRE_MFA=1, anyone without a second factor can only reach /account to set one up.
export async function proxy(req: NextRequest) {
  const secret = process.env.CONSOLE_SESSION_SECRET ?? "";
  const s = await readSession(secret, req.cookies.get(COOKIE)?.value);
  if (s) {
    if (process.env.CONSOLE_REQUIRE_MFA === "1" && !s.m && !req.nextUrl.pathname.startsWith("/account")) {
      const url = req.nextUrl.clone();
      url.pathname = "/account";
      url.search = "";
      return NextResponse.redirect(url);
    }
    if (s.r === "cashier" && !tillOnly(req.nextUrl.pathname)) {
      const url = req.nextUrl.clone();
      url.pathname = "/closing";
      url.search = "";
      return NextResponse.redirect(url);
    }
    return NextResponse.next();
  }
  const url = req.nextUrl.clone();
  url.pathname = "/login";
  url.search = req.nextUrl.pathname === "/" ? "" : `?next=${encodeURIComponent(req.nextUrl.pathname + req.nextUrl.search)}`;
  return NextResponse.redirect(url);
}

export const config = { matcher: ["/((?!login|signup|accept|forgot|reset|verify|check-email|_next/|favicon.svg).*)"] };
