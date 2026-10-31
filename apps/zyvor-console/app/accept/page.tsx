import { AuthShell } from "@/components/AuthShell";
import { PasswordForm } from "@/components/AuthForms";
import { publicPost } from "@/lib/public";

export default async function Accept({ searchParams }: { searchParams: Promise<{ token?: string }> }) {
  const { token } = await searchParams;
  const who = token ? await publicPost<{ email: string; organisation: string; role: string }>("/v1/invitations/lookup", { token }) : null;
  if (!token || !who?.ok) {
    return <AuthShell title="This invitation cannot be used" lede="It may have expired, been cancelled, or already been accepted. Ask the person who invited you to send a new one."><p><a className="btn" href="/login">Go to sign in</a></p></AuthShell>;
  }
  return (
    <AuthShell title={`Join ${who.body.organisation}`} lede={`You were invited as ${who.body.role} with ${who.body.email}. Choose a password to finish.`}>
      <PasswordForm token={token} mode="accept" />
    </AuthShell>
  );
}
