import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { LinkForm } from "@/components/LinkForm";
import { cancelLink } from "@/lib/actions";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

const base = () => (process.env.CHECKOUT_PUBLIC_URL ?? "http://localhost:3002").replace(/\/$/, "");

export default async function Links() {
  const page = await api().paymentLinks.list();
  return (
    <>
      <div className="head"><div><h1>Payment links</h1><p className="lede">One URL, many payers. Each person who opens it gets their own checkout, so a link can collect a fee from a whole class.</p></div></div>
      <LinkForm base={base()} />
      <div className="tablewrap">
        <table>
          <thead><tr><th>Link</th><th className="num">Amount</th><th className="num">Used</th><th>Status</th><th /></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={5} className="empty">No payment links yet.</td></tr>}
            {page.data.map((l) => (
              <tr key={l.id}>
                <td><div>{l.description ?? <span className="muted">Untitled</span>}</div><a className="mono muted" style={{ overflowWrap: "anywhere" }} href={base() + l.path} target="_blank" rel="noreferrer">{base() + l.path}</a></td>
                <td className="num">{l.amount_minor ? inr(l.amount_minor) : <span className="muted">invoice balance</span>}</td>
                <td className="num">{l.uses}{l.max_uses ? ` / ${l.max_uses}` : ""}</td>
                <td><Chip status={l.status} /></td>
                <td>{l.status === "active" && <Act action={cancelLink} id={l.id} label="Cancel" />}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
