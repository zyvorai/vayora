#!/usr/bin/env node
import { Zyvor, ZyvorError, verifyWebhook } from "./index.js";

const HELP = `zyvor <command>   (env: ZYVOR_API_KEY, ZYVOR_API_URL, ZYVOR_ACTOR)

  intents create <amount-rupees> [--external-ref X] [--provider P]
  intents get <intent_id>
  attempts refresh <attempt_id>
  refunds create <intent_id> [<amount-rupees>] [--reason R] [--external-ref X]
  refunds approve <refund_id>
  refunds failed
  settlements explain <settlement_id>
  cases list [--status open]
  webhooks outbox [--status pending]
  webhooks replay <event_id>
  webhooks verify --secret S --header H --body-file F
`;

/** Rupees as typed by a person -> integer paise, without floating point. */
export function rupeesToMinor(s: string): number {
  const m = /^(\d{1,9})(?:\.(\d{1,2}))?$/.exec(s.trim());
  if (!m) throw new Error(`invalid amount "${s}" (use 499 or 499.50)`);
  return Number(m[1]) * 100 + Number((m[2] ?? "").padEnd(2, "0") || 0);
}

function flags(args: string[]): { pos: string[]; f: Record<string, string> } {
  const pos: string[] = [];
  const f: Record<string, string> = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i].startsWith("--")) f[args[i].slice(2)] = args[++i] ?? "";
    else pos.push(args[i]);
  }
  return { pos, f };
}

export async function run(argv: string[], env: Record<string, string | undefined>, out: (s: string) => void, fetchImpl?: typeof fetch): Promise<number> {
  const [group, cmd, ...rest] = argv;
  if (!group || group === "help" || group === "--help") {
    out(HELP);
    return 0;
  }
  const { pos, f } = flags(rest);
  if (group === "webhooks" && cmd === "verify") {
    const { readFileSync } = await import("node:fs");
    const ok = verifyWebhook({ secret: f.secret, header: f.header, rawBody: readFileSync(f["body-file"]) });
    out(ok ? "signature valid" : "signature INVALID");
    return ok ? 0 : 1;
  }
  if (!env.ZYVOR_API_KEY) {
    out("set ZYVOR_API_KEY");
    return 2;
  }
  const z = new Zyvor({ apiKey: env.ZYVOR_API_KEY, baseUrl: env.ZYVOR_API_URL, actor: env.ZYVOR_ACTOR, fetch: fetchImpl });
  const show = (v: unknown) => out(JSON.stringify(v, null, 2));
  try {
    const key = `${group}-${cmd}`;
    if (key === "intents-create") show(await z.intents.create({ amountMinor: rupeesToMinor(pos[0]), externalRef: f["external-ref"], allowedProviders: f.provider ? [f.provider] : undefined }));
    else if (key === "intents-get") show(await z.intents.retrieve(pos[0]));
    else if (key === "attempts-refresh") show(await z.attempts.refresh(pos[0]));
    else if (key === "refunds-create") show(await z.intents.refund(pos[0], { amountMinor: pos[1] ? rupeesToMinor(pos[1]) : undefined, reason: f.reason, externalRef: f["external-ref"] }));
    else if (key === "refunds-approve") show(await z.refunds.approve(pos[0]));
    else if (key === "refunds-failed") show(await z.refunds.list({ status: "failed" }));
    else if (key === "settlements-explain") out((await z.settlements.explain(pos[0])).narrative);
    else if (key === "cases-list") show(await z.cases.list({ status: f.status ?? "open" }));
    else if (key === "webhooks-outbox") show(await z.webhooks.outbox(f.status));
    else if (key === "webhooks-replay") show(await z.webhooks.replay(pos[0]));
    else {
      out(HELP);
      return 2;
    }
    return 0;
  } catch (e) {
    out(e instanceof ZyvorError ? `error ${e.status} ${e.code}: ${e.message}` : String(e instanceof Error ? e.message : e));
    return 1;
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  run(process.argv.slice(2), process.env, (s) => console.log(s)).then((c) => process.exit(c));
}
