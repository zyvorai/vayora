# zyvor (Node SDK and CLI)

Typed client for the Zyvor API. Amounts are whole paise. Everything here is exercised against simulators only.

## Fee checks, bank statements and routing health

```ts
import { Zyvor } from "@zyvor/node";
const z = new Zyvor({ apiKey: "zk_test_demo", baseUrl: "http://localhost:8080" });

// Enter the rate from your own contract, then compare what a provider deducted.
await z.feeRules.create({ provider: "razorpay", method: "upi", bps: 200, effectiveFrom: "2026-04-01", gstBps: 1800 });
await z.feeChecks.updateSettings({ toleranceMinor: 100 });
await z.feeChecks.run({ since: "2026-04-01" });
const findings = await z.feeChecks.findings({ status: "open", limit: 20 });
const one = await z.feeChecks.settlement("set_demo_1"); // read only; recheckSettlement() records the finding

// Dry-run a statement before importing it, then match.
const csv = "Date,Narration,Debit,Credit\n01/04/2026,DEMO CREDIT,,1000.00\n";
const check = await z.bankStatements.preview(csv, { layout: "generic" });
if (check.ok) await z.bankStatements.importCsv(csv, { layout: "generic" });
const lines = await z.bankStatements.lines({ status: "suggested", limit: 50 });
await z.bankStatements.confirmMatch(9, "checked against the settlement report");
const ageing = await z.cases.ageing();

// Provider health (read only) and its thresholds (admin scope).
const health = await z.routing.health("upi");
await z.routing.updateHealthSettings({ minSamples: 20 });
```

Lists take `limit`; none of these endpoints take an idempotency key. Errors are `ZyvorError` with `status` and `code`.

CLI: `zyvor recon ageing`, `zyvor fee-rules list`, `zyvor fee-checks summary|findings|run`, `zyvor bank lines|rematch`, `zyvor routing health`.
