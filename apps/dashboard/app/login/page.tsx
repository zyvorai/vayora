import { LoginForm } from "@/components/LoginForm";

export default async function Login({ searchParams }: { searchParams: Promise<{ next?: string }> }) {
  const { next } = await searchParams;
  return (
    <div className="login">
      <div className="card">
        <img src="/favicon.svg" alt="" width="40" height="40" style={{ borderRadius: 10 }} />
        <h1>Vayora Dashboard</h1>
        <p className="lede">Sign in to view payments, refunds and webhooks.</p>
        <LoginForm next={next ?? "/"} />
      </div>
    </div>
  );
}
