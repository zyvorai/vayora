const map: Record<string, string> = {
  captured: "ok", succeeded: "ok", paid: "ok", processed: "ok", delivered: "ok", resolved: "ok", won: "ok", active: "ok", invoiced: "ok", kept: "ok",
  failed: "bad", dead: "bad", rejected: "bad", lost: "bad", under_review: "accent", broken: "bad", canceled: "bad", void: "bad", unknown: "bad",
  approved: "ok", revoked: "bad", pass: "ok", fail: "bad", inconclusive: "warn", in_review: "warn",
  pending: "warn", requires_payment_method: "warn", requires_action: "warn", pending_approval: "warn", processing: "warn", open: "warn", partially_paid: "warn", paused: "warn", assigned: "accent", created: "accent",
};
/** The words a person would use for a status the API names for developers. */
const WORDS: Record<string, string> = { requires_payment_method: "awaiting payment", requires_action: "needs the payer" };
export const statusLabel = (status: string) => WORDS[status] ?? status.replace(/_/g, " ");

/** Status is never colour alone: dot + label. */
export function Chip({ status }: { status: string }) {
  return <span className={`chip ${map[status] ?? ""}`}>{statusLabel(status)}</span>;
}
