/** "due in 5h", "overdue by 3h": how a request stands against its target, in whole hours (days past 48h). */
export function dueLabel(dueAt: string, now = Date.now()): { text: string; late: boolean } {
  const ms = new Date(dueAt).getTime() - now;
  if (!Number.isFinite(ms)) return { text: "", late: false }; // no date to compare: say nothing rather than guess
  const h = Math.floor(Math.abs(ms) / 3_600_000);
  const span = h >= 48 ? `${Math.floor(h / 24)}d` : h >= 1 ? `${h}h` : "under 1h";
  return ms < 0 ? { text: `overdue by ${span}`, late: true } : { text: `due in ${span}`, late: false };
}
