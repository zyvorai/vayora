import { AuthShell } from "@/components/AuthShell";
import { publicPost } from "@/lib/public";

// Confirming twice is harmless, so this can run when the page loads (mail scanners that open links do no harm).
export default async function Verify({ searchParams }: { searchParams: Promise<{ token?: string }> }) {
  const { token } = await searchParams;
  const r = token ? await publicPost("/v1/signup/verify", { token }) : null;
  if (r?.ok) return <AuthShell title="Email confirmed" lede="Thank you. You can sign in now."><p><a className="btn primary" href="/login">Sign in</a></p></AuthShell>;
  return <AuthShell title="This link cannot be used" lede="It may have expired, or a newer one was sent. Sign in and we will send another."><p><a className="btn" href="/login">Go to sign in</a></p></AuthShell>;
}
