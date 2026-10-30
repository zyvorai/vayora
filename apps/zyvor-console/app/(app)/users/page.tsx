import { redirect } from "next/navigation";
import { Act } from "@/components/Act";
import { UserForm } from "@/components/UserForm";
import { disableUser, enableUser, resetUserMfa } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { istDateTime } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function People() {
  const me = await currentSession();
  if (me?.r !== "admin") redirect("/");
  const [page, branches] = await Promise.all([api().consoleUsers.list(), api().branches.list()]);
  return (
    <>
      <div className="head"><div><h1>People</h1><p className="lede">Who can sign in. <strong>Viewers</strong> can look, <strong>finance</strong> can also move money and send reminders, <strong>admins</strong> manage people.</p></div></div>
      <UserForm branches={branches.data.map((b) => ({ id: b.id, name: b.name }))} />
      <div className="tablewrap">
        <table>
          <thead><tr><th>User</th><th>Role</th><th>Second factor</th><th>Last sign-in (IST)</th><th>Status</th><th /></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={6} className="empty">No named users yet. The deployment admin can still sign in until you add one.</td></tr>}
            {page.data.map((u) => (
              <tr key={u.id}>
                <td>{u.username}</td><td>{u.role}{u.branch_id && <span className="muted"> · {branches.data.find((b) => b.id === u.branch_id)?.name ?? u.branch_id}</span>}</td><td>{u.mfa_enrolled ? "On" : <span className="muted">Off</span>}</td>
                <td className="muted">{u.last_login_at ? istDateTime(u.last_login_at) : "never"}</td>
                <td>{u.disabled ? "Disabled" : u.locked ? "Locked" : "Active"}</td>
                <td style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
                  {(u.disabled || u.locked) && <Act action={enableUser} id={u.id} label={u.disabled ? "Enable" : "Unlock"} />}
                  {!u.disabled && u.id !== me.i && <Act action={disableUser} id={u.id} label="Disable" />}
                  {u.mfa_enrolled && <Act action={resetUserMfa} id={u.id} label="Reset 2FA" />}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
