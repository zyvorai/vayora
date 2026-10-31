import { AuthShell } from "@/components/AuthShell";
import { ResendForm } from "@/components/AuthForms";

export default async function CheckEmail({ searchParams }: { searchParams: Promise<{ email?: string }> }) {
  const { email } = await searchParams;
  return (
    <AuthShell title="Check your email" lede={`We sent a confirmation link${email ? ` to ${email}` : ""}. Open it, then sign in. It can take a minute to arrive.`}>
      {email && <ResendForm email={email} />}
      <p className="fine" style={{ textAlign: "center", marginTop: 14 }}><a href="/login">Back to sign in</a></p>
    </AuthShell>
  );
}
