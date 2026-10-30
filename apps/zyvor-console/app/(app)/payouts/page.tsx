import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { AddVendor, BulkPayouts, MarkPaid, RequestPayout } from "@/components/PayoutForms";
import { approvePayout, cancelPayout, rejectPayout } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function Payouts() {
  const me = await currentSession();
  const z = api();
  const [payouts, vendors] = await Promise.all([z.payouts.list(), z.payouts.vendors()]);
  const name = new Map(vendors.data.map((v) => [v.id, v.name]));
  // what each vendor is owed from split payments (a few dozen vendors at most on this page)
  const balances = await Promise.all(vendors.data.slice(0, 50).map(async (v) => [v.id, (await z.payouts.vendorBalance(v.id)).balances] as const));
  const owed = new Map(balances);
  const waiting = payouts.data.filter((p) => p.status === "pending_approval").length;
  return (
    <>
      <div className="head"><div><h1>Payouts</h1><p className="lede">Money you send to vendors. {waiting ? `${waiting} waiting for a second person.` : "Every payout needs a second person to approve it."} You make the transfer from your own bank, then record its reference here.</p></div></div>
      <RequestPayout vendors={vendors.data.filter((v) => !v.archived)} />
      <BulkPayouts />
      <AddVendor />
      {balances.some(([, b]) => b.length > 0) && (
        <>
          <h2>Owed to vendors from split payments</h2>
          <div className="tablewrap" style={{ marginBottom: 24 }}>
            <table>
              <thead><tr><th>Vendor</th><th className="num">Owed</th><th className="num">Free to pay out</th></tr></thead>
              <tbody>
                {vendors.data.filter((v) => (owed.get(v.id) ?? []).length > 0).map((v) => (owed.get(v.id) ?? []).map((b) => (
                  <tr key={v.id + b.currency}><td>{v.name}</td><td className="num">{inr(b.owed_minor)}</td><td className="num">{inr(b.available_minor)}</td></tr>
                )))}
              </tbody>
            </table>
          </div>
          <h2>Payouts</h2>
        </>
      )}
      <div className="tablewrap">
        <table>
          <thead><tr><th>Vendor</th><th>Purpose</th><th className="num">Amount</th><th>Requested by</th><th>Status</th><th /></tr></thead>
          <tbody>
            {payouts.data.length === 0 && <tr><td colSpan={6} className="empty">No payouts yet.</td></tr>}
            {payouts.data.map((p) => (
              <tr key={p.id}>
                <td>{name.get(p.vendor_id) ?? p.vendor_id}</td><td>{p.purpose}</td><td className="num">{inr(p.amount_minor)}</td>
                <td className="muted">{p.requested_by.replace(/^user:/, "")}</td>
                <td><Chip status={p.status} />{p.bank_ref && <div className="mono muted">{p.bank_ref}</div>}</td>
                <td style={{ display: "flex", gap: 8, justifyContent: "flex-end", alignItems: "center" }}>
                  {p.status === "pending_approval" && p.requested_by !== `user:${me?.u}` && <><Act action={approvePayout} id={p.id} label="Approve" primary /><Act action={rejectPayout} id={p.id} label="Reject" /></>}
                  {p.status === "pending_approval" && p.requested_by === `user:${me?.u}` && <span className="muted">Waiting for someone else</span>}
                  {p.status === "approved" && <MarkPaid id={p.id} />}
                  {(p.status === "pending_approval" || p.status === "approved") && <Act action={cancelPayout} id={p.id} label="Cancel" />}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
