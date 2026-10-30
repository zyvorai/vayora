import "server-only";
import { cookies } from "next/headers";
import { COOKIE, readSession, type Session } from "@/lib/session";

/** Who is signed in, from the signed cookie. `null` when nobody is (the proxy keeps those out of the app). */
export async function currentSession(): Promise<Session | null> {
  return readSession(process.env.CONSOLE_SESSION_SECRET ?? "", (await cookies()).get(COOKIE)?.value);
}
