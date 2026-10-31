import { Act } from "@/components/Act";
import { KeyForm } from "@/components/KeyForm";
import { revokeKey } from "@/lib/actions";
import { requireRole } from "@/lib/auth";
import { istDateTime } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function Keys() {
  await requireRole("admin");
  const z = await api();
  const [keys, branches] = await Promise.all([z.keys.list(), z.branches.list()]);
  const branch = new Map(branches.data.map((b) => [b.id, b.name]));
  return (
    <>
      <div className="head"><div><h1>API keys</h1><p className="lede">Keys for tills, apps and integrations. A key limited to a branch can only take and see that branch’s payments. The secret is shown once, when you create it.</p></div></div>
      <KeyForm branches={branches.data} />
      <div className="tablewrap"><table>
        <thead><tr><th>Name</th><th>Key</th><th>Limited to</th><th>Can</th><th>Last used (IST)</th><th>Status</th><th /></tr></thead>
        <tbody>
          {keys.data.map((k) => {
            const gone = !!k.revoked_at || (!!k.expires_at && new Date(k.expires_at) < new Date());
            return (
              <tr key={k.id}>
                <td>{k.name}</td><td className="mono">{k.prefix}…</td>
                <td>{k.branch_id ? branch.get(k.branch_id) ?? k.branch_id : <span className="muted">Whole business</span>}</td>
                <td className="muted">{k.scopes.includes("admin") ? "Everything" : k.scopes.map((s) => s.replace(":", " ")).join(", ")}</td>
                <td className="muted">{k.last_used_at ? istDateTime(k.last_used_at) : "Never"}</td>
                <td>{k.revoked_at ? "Revoked" : gone ? "Expired" : "Active"}</td>
                <td>{!gone && <Act action={revokeKey} id={k.id} label="Revoke" />}</td>
              </tr>
            );
          })}
        </tbody>
      </table></div>
    </>
  );
}
