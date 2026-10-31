import { AuthShell } from "@/components/AuthShell";
import { PasswordForm } from "@/components/AuthForms";

export default async function Reset({ searchParams }: { searchParams: Promise<{ token?: string }> }) {
  const { token } = await searchParams;
  if (!token) return <AuthShell title="This link is not complete" lede="Open the link from your email again, or ask for a new one."><p><a className="btn primary" href="/forgot">Send a new link</a></p></AuthShell>;
  return <AuthShell title="Choose a new password" lede="You will be signed out everywhere else."><PasswordForm token={token} mode="reset" /></AuthShell>;
}
