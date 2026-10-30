/** One payout per line: `vendor, amount, purpose, reference`. The vendor is a name or id; amounts are in rupees. */
export function parseBulk(csv: string, vendors: { id: string; name: string }[]): { items: { vendorId: string; amountMinor: number; purpose: string; externalRef: string }[] } | { error: string } {
  const byName = new Map(vendors.map((v) => [v.name.trim().toLowerCase(), v.id]));
  const ids = new Set(vendors.map((v) => v.id));
  const items: { vendorId: string; amountMinor: number; purpose: string; externalRef: string }[] = [];
  // line numbers are the ones the person sees, counting blank and comment lines
  for (const [i, raw] of csv.split(/\r?\n/).entries()) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const cols = line.split(",").map((c) => c.trim());
    // a comma inside an amount (12,500) would shift every column and pay the wrong sum, so refuse rather than guess
    if (cols.length > 4) return { error: `Line ${i + 1}: more than four columns. Write amounts without commas (12500, not 12,500).` };
    const [vendor, amount, purpose, ref] = cols;
    if (items.length === 0 && /^vendor$/i.test(cols[0] ?? "")) continue; // a header row
    const id = ids.has(vendor ?? "") ? vendor : byName.get((vendor ?? "").toLowerCase());
    const rupees = Number((amount ?? "").replace(/[₹\s]/g, ""));
    if (!id) return { error: `Line ${i + 1}: no vendor called "${vendor}".` };
    if (!Number.isFinite(rupees) || rupees <= 0) return { error: `Line ${i + 1}: "${amount}" is not an amount.` };
    if (!purpose) return { error: `Line ${i + 1}: add a purpose.` };
    if (!ref) return { error: `Line ${i + 1}: add a reference so repeating the upload cannot pay twice.` };
    items.push({ vendorId: id, amountMinor: Math.round(rupees * 100), purpose, externalRef: ref });
  }
  if (items.length === 0) return { error: "Paste at least one line." };
  if (items.length > 200) return { error: "At most 200 payouts at a time." };
  return { items };
}

