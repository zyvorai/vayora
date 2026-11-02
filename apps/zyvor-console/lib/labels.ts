/** The names people know, for the short internal ids the API uses. */
const PROVIDERS: Record<string, string> = { razorpay: "Razorpay", cashfree: "Cashfree", sim: "Simulator (test)", custom: "Your own gateway" };
const METHODS: Record<string, string> = { upi: "UPI", card: "Card", netbanking: "Net banking", wallet: "Wallet", cash: "Cash" };

export const providerLabel = (id: string) => PROVIDERS[id] ?? id;
export const methodLabel = (id: string) => METHODS[id] ?? id.replace(/_/g, " ");
