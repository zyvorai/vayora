import { LoginForm } from "@/components/LoginForm";

export default async function Login({ searchParams }: { searchParams: Promise<{ next?: string; reset?: string }> }) {
  const { next, reset } = await searchParams;
  return (
    <div className="login">
      <div className="aurora" aria-hidden="true" />
      <div className="card">
        <img src="/favicon.svg" alt="" width="44" height="44" style={{ borderRadius: 11 }} />
        <h1>Welcome to Zyvor</h1>
        <p className="lede">Payments, billing and reconciliation in one place.</p>
        <LoginForm next={next ?? "/"} reset={reset === "1"} />
        <p className="fine">Every action here is recorded in the audit trail.</p>
      </div>
    </div>
  );
}
