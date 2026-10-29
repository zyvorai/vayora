const map: Record<string, string> = {
  captured: "ok", paid: "ok", processed: "ok", delivered: "ok",
  failed: "bad", dead: "bad",
  pending: "warn", created: "accent",
};
/** Status is never colour alone: dot + label. */
export function Chip({ status }: { status: string }) {
  return <span className={`chip ${map[status] ?? ""}`}>{status}</span>;
}
