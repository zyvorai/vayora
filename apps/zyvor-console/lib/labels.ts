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
