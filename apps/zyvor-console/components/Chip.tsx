const map: Record<string, string> = {
  captured: "ok", succeeded: "ok", paid: "ok", processed: "ok", delivered: "ok", resolved: "ok", active: "ok", invoiced: "ok", kept: "ok",
  failed: "bad", dead: "bad", rejected: "bad", broken: "bad", canceled: "bad", void: "bad", unknown: "bad",
  pending: "warn", pending_approval: "warn", processing: "warn", open: "warn", partially_paid: "warn", paused: "warn", assigned: "accent", created: "accent",
};
/** Status is never colour alone: dot + label. */
export function Chip({ status }: { status: string }) {
  return <span className={`chip ${map[status] ?? ""}`}>{status.replace(/_/g, " ")}</span>;
}
