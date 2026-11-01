// Shared by the reviewer rota page and its form (no server-only imports, so a client component can use it).
export const DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;
export type RotaShift = { weekday: (typeof DAYS)[number]; start: string; end: string };
export type RotaView = {
  configured: boolean; utc_offset_minutes: number; local_now: string; on_duty_now: string[];
  reviewers: { id: string; name: string; me: boolean; on_duty: boolean; shifts: RotaShift[] }[];
};

/** `330` as `UTC+5:30`. */
export const offsetLabel = (m: number) => `UTC${m < 0 ? "-" : "+"}${Math.floor(Math.abs(m) / 60)}${Math.abs(m) % 60 ? ":" + String(Math.abs(m) % 60).padStart(2, "0") : ""}`;
