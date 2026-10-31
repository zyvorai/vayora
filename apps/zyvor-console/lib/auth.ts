import "server-only";
import { cookies } from "next/headers";
import { redirect } from "next/navigation";
import { COOKIE, allows, readSession, type Role, type Session } from "@/lib/session";

/** Who is signed in, from the signed cookie. `null` when nobody is (the proxy keeps those out of the app). */
export async function currentSession(): Promise<Session | null> {
  return readSession(process.env.CONSOLE_SESSION_SECRET ?? "", (await cookies()).get(COOKIE)?.value);
}

/** Pages the menu hides from a role should also refuse it when the address is typed in: back to the overview. */
export async function requireRole(need: Role): Promise<Session> {
  const s = await currentSession();
  if (!s || !allows(s.r, need)) redirect("/");
  return s;
}
