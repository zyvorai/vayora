import { SignupForm } from "@/components/SignupForm";

export default function Signup() {
  return (
    <div className="login">
      <div className="aurora" aria-hidden="true" />
      <div className="card">
        <img src="/favicon.svg" alt="" width="44" height="44" style={{ borderRadius: 11 }} />
        <h1>Create your Zyvor account</h1>
        <p className="lede">Start in test mode right away. Live payments open after your business is verified.</p>
        <SignupForm />
      </div>
    </div>
  );
}
