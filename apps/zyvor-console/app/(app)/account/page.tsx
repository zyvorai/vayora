import { MfaSetup } from "@/components/MfaSetup";
import { currentSession } from "@/lib/auth";

export default async function Account() {
  const me = await currentSession();
  const required = process.env.CONSOLE_REQUIRE_MFA === "1";
  return (
    <>
      <div className="head"><div><h1>Account</h1><p className="lede">Signed in as <strong>{me?.u}</strong> ({me?.r}).</p></div></div>
      <div className="card" style={{ maxWidth: 560 }}>
        <h2>Second factor</h2>
        {me?.i === "bootstrap" ? (
          <p>You are signed in with the deployment admin. Add a named user under People, sign in as them, and turn on a second factor there.</p>
        ) : (
          <>
            {!me?.m && <p>{required ? "Your organisation requires a second factor. Set one up to continue." : "Add an authenticator app (any TOTP app) so a stolen password is not enough to get in."}</p>}
            <MfaSetup enrolled={me?.m === true} />
          </>
        )}
      </div>
    </>
  );
}
