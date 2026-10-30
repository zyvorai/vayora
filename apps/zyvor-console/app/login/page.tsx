import { LoginForm } from "@/components/LoginForm";

export default async function Login({ searchParams }: { searchParams: Promise<{ next?: string }> }) {
  const { next } = await searchParams;
  return (
    <div className="login">
      <div className="aurora" aria-hidden="true" />
      <div className="card">
        <img src="/favicon.svg" alt="" width="44" height="44" style={{ borderRadius: 11 }} />
        <h1>Welcome to Zyvor</h1>
        <p className="lede">Payments, billing and reconciliation in one place.</p>
        <LoginForm next={next ?? "/"} />
        <p className="fine">Every action here is recorded in the audit trail.</p>
      </div>
    </div>
  );
}
