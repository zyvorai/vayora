import { redirect } from "next/navigation";
import { SettingsForm } from "@/components/SettingsForm";
import { saveApprovals, saveCollections, saveFxRate, saveGst, saveRisk, saveWebhooks } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { api } from "@/lib/zyvor";

const major = (minor: number) => (minor / 100).toFixed(2);

export default async function Settings() {
  const me = await currentSession();
  if (me?.r !== "admin") redirect("/");
  const z = api();
  const [s, cat, fx] = await Promise.all([z.settings.get(), z.webhooks.catalogue(), z.fx.rates()]);
  const all = s.webhook_events.includes("*");
  const chosen = (t: string) => s.webhook_events.some((p) => p === t || (p.endsWith(".*") && t.startsWith(p.slice(0, -1))));
  return (
    <>
      <div className="head"><div><h1>Settings</h1><p className="lede">How reminders, approvals, tax and webhooks behave for this {s.env} environment.</p></div></div>

      <SettingsForm title="Collections" hint="Remind customers about overdue invoices on a schedule. Nothing is sent by Zyvor itself: your own messaging picks up the reminder event." action={saveCollections}>
        <label className="field checkbox"><input type="checkbox" name="enabled" defaultChecked={s.dunning.enabled} /> Remind automatically</label>
        <label className="field checkbox"><input type="checkbox" name="requires_approval" defaultChecked={s.dunning.requires_approval} /> A person approves each reminder first</label>
        <label className="field">Channel
          <select name="channel" defaultValue={s.dunning.channel}>{["whatsapp", "email", "sms", "call", "other"].map((c) => <option key={c} value={c}>{c}</option>)}</select>
        </label>
        <label className="field">Days overdue before the first<input name="min_days" type="number" min={0} defaultValue={s.dunning.min_days_overdue} /></label>
        <label className="field">Days between reminders<input name="repeat_days" type="number" min={0} max={365} defaultValue={s.dunning.repeat_after_days} /></label>
      </SettingsForm>

      <SettingsForm title="Approvals" hint="Amounts above these need a second person. Set payouts to 0 to require one for every payout." action={saveApprovals}>
        <label className="field">Refunds above (₹)<input name="refund" inputMode="decimal" defaultValue={major(s.refund_approval_threshold_minor)} /></label>
        <label className="field">Payouts above (₹)<input name="payout" inputMode="decimal" defaultValue={major(s.payout_approval_threshold_minor)} /></label>
      </SettingsForm>

      <SettingsForm title="Risk limits" hint="Payments that break these are stopped before they reach a provider, and each stop is recorded. Leave a box empty for no limit." action={saveRisk}>
        <label className="field">Largest single payment (₹)<input name="max_amount" inputMode="decimal" defaultValue={s.risk.max_intent_amount_minor ? major(s.risk.max_intent_amount_minor) : ""} /></label>
        <label className="field">Tries per payment<input name="max_attempts" type="number" min={1} defaultValue={s.risk.max_attempts_per_intent ?? ""} /></label>
        <label className="field">Failed tries per customer per hour<input name="max_failed" type="number" min={1} defaultValue={s.risk.max_failed_per_customer_per_hour ?? ""} /></label>
        <label className="field" style={{ gridColumn: "1 / -1" }}>Refuse these UPI ids or handles (one per line, e.g. name@bank or @bank)
          <textarea name="blocked" rows={3} defaultValue={(s.risk.blocked_vpas ?? []).join("\n")} spellCheck={false} />
        </label>
      </SettingsForm>

      <SettingsForm title="GST" hint="Needed to issue tax invoices. The GSTIN is checked, including its check character." action={saveGst}>
        <label className="field">GSTIN<input name="gstin" defaultValue={s.gst.gstin ?? ""} maxLength={15} autoCapitalize="characters" spellCheck={false} /></label>
        <label className="field">Legal name<input name="legal_name" defaultValue={s.gst.legal_name ?? ""} /></label>
      </SettingsForm>

      <SettingsForm title="Exchange rate" hint={`Rupees per US dollar, used to show dollar amounts in INR in reports. One rate per day; it is never edited, so enter the next day's to change it.${fx.data[0] ? ` Latest: ${fx.data[0].rate} on ${fx.data[0].as_of}.` : " None recorded yet."}`} action={saveFxRate}>
        <label className="field">₹ per US$<input name="rate" inputMode="decimal" placeholder="83.2150" defaultValue="" /></label>
      </SettingsForm>

      <SettingsForm title="Webhooks" hint="Where events are sent and which ones. The signing secret is write-only: leave it blank to keep the current one." action={saveWebhooks}>
        <label className="field" style={{ gridColumn: "1 / -1" }}>Endpoint URL<input name="url" type="url" defaultValue={s.outbound_webhook_url ?? ""} placeholder="https://example.com/hooks/zyvor" /></label>
        <label className="field" style={{ gridColumn: "1 / -1" }}>Signing secret<input name="secret" type="password" autoComplete="new-password" placeholder="unchanged" /></label>
        <label className="field checkbox" style={{ gridColumn: "1 / -1" }}><input type="checkbox" name="all" defaultChecked={all} /> Send every event</label>
        {cat.data.map((e) => (
          <label key={e.type} className="field checkbox" title={e.description}>
            <input type="checkbox" name="event" value={e.type} defaultChecked={!all && chosen(e.type)} /> <span className="mono">{e.type}</span>
          </label>
        ))}
      </SettingsForm>
    </>
  );
}
