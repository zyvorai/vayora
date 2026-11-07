import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { openSettlementBatch, releaseSettlementBatch, revokeCardToken } from "@/lib/actions";
import { inr, istDateTime } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function Settle() {
  const z = await api();
  const [batches, proof, tokens, money] = await Promise.all([
    z.settlementBatches.list().catch(() => ({ data: [] as Record<string, unknown>[] })),
    z.escrow.proof().catch(() => null),
    z.cardTokens.list().catch(() => ({ tokens: [] as { id: string; last4: string; brand: string | null }[] })),
    z.moneyMode.get().catch(() => null),
  ]);
  const mode = money?.money_mode ?? "orchestrate";
  const rows = batches.data ?? [];
  return (
    <>
      <div className="head">
        <div>
          <h1>Settlement</h1>
          <p className="lede">
            {mode === "acquire"
              ? "Acquire mode: captures enrol into open batches; release after the settlement delay pays out via the sponsor rail."
              : "Settlement batches are for acquire mode. Switch money mode in Settings to use escrow batches."}
          </p>
        </div>
      </div>

      {proof && (
        <div className="tablewrap" style={{ marginBottom: 24 }}>
          <table>
            <thead><tr><th>Escrow available</th><th>Open payables</th><th>Balanced</th></tr></thead>
            <tbody>
              <tr>
                <td className="num">{inr(proof.escrow_available_net)}</td>
                <td className="num">{inr(proof.open_payables_net)}</td>
                <td>{proof.balanced ? <Chip status="balanced" /> : <Chip status="unbalanced" />}</td>
              </tr>
            </tbody>
          </table>
        </div>
      )}

      {mode === "acquire" && (
        <div style={{ marginBottom: 24 }}>
          <Act action={openSettlementBatch} label="Open INR batch" primary />
        </div>
      )}

      <h2>Batches</h2>
      <div className="tablewrap" style={{ marginBottom: 32 }}>
        <table>
          <thead>
            <tr>
              <th>Opened</th><th>Currency</th><th className="num">Captures</th><th className="num">Refunds</th>
              <th className="num">Fees</th><th className="num">Net</th><th>Status</th><th />
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr><td colSpan={8} className="empty">No settlement batches yet. In acquire mode a batch opens when the first capture enrols.</td></tr>
            )}
            {rows.map((b) => {
              const id = String(b.id ?? "");
              const status = String(b.status ?? "");
              return (
                <tr key={id}>
                  <td className="muted" style={{ whiteSpace: "nowrap" }}>{b.created_at ? istDateTime(String(b.created_at)) : "—"}</td>
                  <td>{String(b.currency ?? "")}</td>
                  <td className="num">{inr(Number(b.captures_minor ?? 0))}</td>
                  <td className="num">{inr(Number(b.refunds_minor ?? 0))}</td>
                  <td className="num">{inr(Number(b.fees_minor ?? 0))}</td>
                  <td className="num">{inr(Number(b.net_minor ?? 0))}</td>
                  <td><Chip status={status} />{b.rail_ref ? <div className="mono muted">{String(b.rail_ref)}</div> : null}</td>
                  <td style={{ textAlign: "right" }}>
                    {status === "open" && Number(b.net_minor ?? 0) > 0 && (
                      <Act action={releaseSettlementBatch} id={id} label="Release" primary />
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <h2>Network card tokens</h2>
      <p className="lede" style={{ marginTop: 0 }}>Stored network-token refs only — never PAN or CVV. Charge with <code>card_token_id</code> on an attempt.</p>
      <div className="tablewrap">
        <table>
          <thead><tr><th>Token</th><th>Last4</th><th>Brand</th><th /></tr></thead>
          <tbody>
            {(tokens.tokens ?? []).length === 0 && (
              <tr><td colSpan={4} className="empty">No card tokens stored.</td></tr>
            )}
            {(tokens.tokens ?? []).map((t) => (
              <tr key={t.id}>
                <td className="mono">{t.id.slice(0, 14)}…</td>
                <td>•••• {t.last4}</td>
                <td>{t.brand ?? "—"}</td>
                <td style={{ textAlign: "right" }}><Act action={revokeCardToken} id={t.id} label="Revoke" /></td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
