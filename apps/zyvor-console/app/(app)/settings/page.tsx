import { redirect } from "next/navigation";
import { SettingsForm } from "@/components/SettingsForm";
import { slug } from "@/lib/labels";
import { resetTestData, saveApprovals, saveCollections, saveFxRate, saveGst, saveReceipts, saveRisk, saveWebhooks } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { api } from "@/lib/zyvor";

const major = (minor: number) => (minor / 100).toFixed(2);

export default async function Settings() {
  const me = await currentSession();
  if (me?.r !== "admin") redirect("/");
  const z = await api();
  const [s, cat, fx] = await Promise.all([z.settings.get(), z.webhooks.catalogue(), z.fx.rates()]);
  const all = s.webhook_events.includes("*");
  const chosen = (t: string) => s.webhook_events.some((p) => p === t || (p.endsWith(".*") && t.startsWith(p.slice(0, -1))));
  return (
    <>
      <div className="head"><div><h1>Settings</h1><p className="lede">How reminders, approvals, tax and webhooks behave for this {s.env} environment.</p></div></div>
      <nav aria-label="On this page" style={{ marginBottom: 20 }}>
        <div className="seg">
          {[["Collections", "Collections"], ["Receipts, invoices, reminders and replies", "Emails"], ["Approvals", "Approvals"], ["Risk limits", "Risk"], ["GST", "GST"], ["Exchange rate", "Exchange rate"], ["Webhooks", "Webhooks"], ...(s.env === "test" ? [["Test data", "Test data"]] : [])].map(([title, label]) => <a key={title} href={`#${slug(title)}`}>{label}</a>)}
        </div>
      </nav>

      <SettingsForm title="Collections" hint="Remind customers about overdue invoices on a schedule. Each reminder is logged and announced to your own messaging (the reminder event); Zyvor emails it itself only when reminder emails are on below and the channel is email." action={saveCollections}>
        <label className="field checkbox"><input type="checkbox" name="enabled" defaultChecked={s.dunning.enabled} /> Remind automatically</label>
        <label className="field checkbox"><input type="checkbox" name="requires_approval" defaultChecked={s.dunning.requires_approval} /> A person approves each reminder first</label>
        <label className="field">Channel
          <select name="channel" defaultValue={s.dunning.channel}>{["whatsapp", "email", "sms", "call", "other"].map((c) => <option key={c} value={c}>{c}</option>)}</select>
        </label>
        <label className="field">Days overdue before the first<input name="min_days" type="number" min={0} defaultValue={s.dunning.min_days_overdue} /></label>
        <label className="field">Days between reminders<input name="repeat_days" type="number" min={0} max={365} defaultValue={s.dunning.repeat_after_days} /></label>
      </SettingsForm>

      <SettingsForm title="Receipts, invoices, reminders and replies" hint="Email the payer a receipt when their payment is captured, and another when a refund goes through. It goes to the email address on the customer, so a payment with no customer, or a customer with no email, sends nothing. Needs outgoing mail to be set up. When a payer replies to a receipt or a payment-link email, the reply goes to the address below instead of to Zyvor." action={saveReceipts}>
        <label className="field checkbox"><input type="checkbox" name="receipts" defaultChecked={s.payment_receipts} /> Email payment and refund receipts</label>
        <label className="field checkbox"><input type="checkbox" name="reminders" defaultChecked={s.reminder_emails} /> <span>Email payment reminders<small className="muted" style={{ display: "block" }}>Only reminders on the email channel. Leave off if your own system already sends them.</small></span></label>
        <label className="field checkbox"><input type="checkbox" name="invoices" defaultChecked={s.invoice_emails} /> <span>Email invoices when billing raises them<small className="muted" style={{ display: "block" }}>Each subscription invoice goes to its customer, with what it is made of and the amount due. A customer with no email gets nothing. You can also email any invoice by hand from Billing.</small></span></label>
        <label className="field">Replies go to<input name="reply_to" type="email" maxLength={200} placeholder="accounts@yourbusiness.example" defaultValue={s.reply_to_email ?? ""} /></label>
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

      <SettingsForm title="GST" hint="Needed to issue tax invoices. The GSTIN is checked, including its check character. The address appears on printed invoices and the draft e-invoice." action={saveGst}>
        <label className="field">GSTIN<input name="gstin" defaultValue={s.gst.gstin ?? ""} maxLength={15} autoCapitalize="characters" spellCheck={false} /></label>
        <label className="field">Legal name<input name="legal_name" defaultValue={s.gst.legal_name ?? ""} /></label>
        <label className="field">Address<input name="address1" defaultValue={s.gst.address1 ?? ""} maxLength={100} /></label>
        <label className="field">City<input name="location" defaultValue={s.gst.location ?? ""} maxLength={50} /></label>
        <label className="field">Pincode<input name="pincode" defaultValue={s.gst.pincode ?? ""} inputMode="numeric" maxLength={6} /></label>
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

      {s.env === "test" && (
        <SettingsForm title="Test data" hint="Clear the payments, invoices, customers, refunds and ledger created while trying things out, so you can start again. Your settings, providers, API keys, people, branches, vendors, plans and coupons stay. The audit trail is kept and records the reset. This cannot be undone, and it is not available in the live environment." action={resetTestData} button="Reset test data" busy="Resetting…" done="Test data cleared.">
          <label className="field" style={{ gridColumn: "1 / -1" }}>Type <span className="mono">reset test data</span> to confirm<input name="confirm" autoComplete="off" spellCheck={false} /></label>
        </SettingsForm>
      )}
    </>
  );
}
