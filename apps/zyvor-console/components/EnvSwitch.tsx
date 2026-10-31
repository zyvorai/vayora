"use client";
import { useActionState } from "react";
import { switchEnv } from "@/lib/actions";

/** Test / Live toggle. Each side is a submit button, so it works without scripts and the page reloads in the new mode. */
export function EnvSwitch({ env }: { env: "test" | "live" }) {
  const [state, action, pending] = useActionState(switchEnv, undefined);
  return (
    <form action={action} className="envsw" aria-label="Environment">
      <div role="group" aria-label="Test or live mode" className="seg">
        {(["test", "live"] as const).map((e) => (
          <button key={e} name="env" value={e} type="submit" aria-pressed={env === e} disabled={pending} className={env === e ? `on ${e}` : ""}>
            {e === "test" ? "Test" : "Live"}
          </button>
        ))}
      </div>
      {state?.error && <div className="note bad envsw-err" role="alert">{state.error}</div>}
    </form>
  );
}
