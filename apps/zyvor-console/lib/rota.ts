// Shared by the reviewer rota page and its form (no server-only imports, so a client component can use it).
export const DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;
export type RotaShift = { weekday: (typeof DAYS)[number]; start: string; end: string };
export type RotaView = {
  configured: boolean; utc_offset_minutes: number; timezone: string | null; local_now: string; local_date: string; on_duty_now: string[];
  holiday_today: string | null;
  i_am_lead: boolean;
  reviewers: { id: string; name: string; me: boolean; lead: boolean; on_duty: boolean; on_leave: boolean; shifts: RotaShift[] }[];
  days_off: DayOff[];
};
/** Leave for one reviewer, or a holiday for the whole desk (`desk`). Days are `YYYY-MM-DD` in the desk's time, inclusive. */
export type DayOff = { removable: boolean; id: string; desk: boolean; reviewer: string | null; mine: boolean; first_day: string; last_day: string; note: string };

/** `330` as `UTC+5:30`. */
export const offsetLabel = (m: number) => `UTC${m < 0 ? "-" : "+"}${Math.floor(Math.abs(m) / 60)}${Math.abs(m) % 60 ? ":" + String(Math.abs(m) % 60).padStart(2, "0") : ""}`;

export type Coverage = { configured: boolean; days: { date: string; weekday: string; holiday: string | null; on_leave: number; gaps: { from: string; to: string }[]; fully_covered: boolean }[] };
