/** The names people know, for the short internal ids the API uses. */
const PROVIDERS: Record<string, string> = { razorpay: "Razorpay", cashfree: "Cashfree", sim: "Simulator (test)", custom: "Your own gateway" };
const METHODS: Record<string, string> = { upi: "UPI", card: "Card", netbanking: "Net banking", wallet: "Wallet", cash: "Cash" };

export const providerLabel = (id: string) => PROVIDERS[id] ?? id;
export const methodLabel = (id: string) => METHODS[id] ?? id.replace(/_/g, " ");

/** Who did something, in words: a person by name, an API key or the system by what it is. */
export function actorLabel(actor: string | null | undefined): string {
  if (!actor) return "—";
  if (actor.startsWith("user:")) return actor.slice(5);
  if (actor.startsWith("key:")) return "API";
  if (actor.startsWith("system:")) return "Automatic";
  return actor;
}

/** "product_not_received" as "Product not received". */
export const words = (id: string) => {
  const t = id.replace(/_/g, " ");
  return t.charAt(0).toUpperCase() + t.slice(1);
};

/** How far off a date is, from today in India, as a phrase and a tone for a chip. */
export function dueIn(day: string): { text: string; tone: "" | "warn" | "bad" } {
  const today = new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Kolkata" }).format(new Date());
  const days = Math.round((Date.parse(day + "T00:00:00Z") - Date.parse(today + "T00:00:00Z")) / 86_400_000);
  if (days < 0) return { text: `${-days} day${days === -1 ? "" : "s"} overdue`, tone: "bad" };
  if (days === 0) return { text: "today", tone: "bad" };
  return { text: `in ${days} day${days === 1 ? "" : "s"}`, tone: days <= 3 ? "warn" : "" };
}
