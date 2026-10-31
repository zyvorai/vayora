/** What a test payment can do. The simulator reads the payer's UPI handle, so each outcome is a handle. */
export const OUTCOMES = [
  { key: "succeed", label: "Succeeds", handle: "ok@bank", note: "Captured at once; the payment can be refunded." },
  { key: "decline", label: "Is declined", handle: "fail@bank", note: "Fails with insufficient funds; the payer can try again." },
  { key: "pending", label: "Stays pending", handle: "pending@bank", note: "Accepted but not finished, as when a payer has not approved the request." },
  { key: "unknown", label: "Outcome unknown", handle: "uncertain@bank", note: "The provider took it but its answer timed out. No new attempt can start until it resolves." },
] as const;

export type OutcomeKey = (typeof OUTCOMES)[number]["key"];

export const handleFor = (key: string): string | null => OUTCOMES.find((o) => o.key === key)?.handle ?? null;

/** The simulator accepts all four methods. It decides the outcome from the handle whatever the method, and no card number
 *  or bank is collected, so these exercise routing, ledger and refunds, not a card form. */
export const METHODS = [
  { key: "upi", label: "UPI" },
  { key: "card", label: "Card" },
  { key: "netbanking", label: "Net banking" },
  { key: "wallet", label: "Wallet" },
] as const;

export type MethodKey = (typeof METHODS)[number]["key"];

export const methodFor = (key: string): MethodKey | null => METHODS.find((m) => m.key === key)?.key ?? null;
