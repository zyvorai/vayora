import { redirect } from "next/navigation";
import { RotaForm } from "@/components/ReviewerForms";
import { platform, requireReviewerSurface, reviewerKey } from "@/lib/reviewer";
import { DAYS, offsetLabel, type RotaView } from "@/lib/rota";

export default async function Rota() {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) redirect("/reviewer");
  const r = await platform<RotaView>(key, "GET", "/rota");
  if (!r.ok) {
    if (r.status === 401) redirect("/reviewer");
    return <div className="note bad" role="alert">{r.message}</div>;
  }
  const v = r.body;
  const mine = v.reviewers.find((x) => x.me);
  // the form edits one shift per day; a rota set through the API with split shifts would be flattened by it, so say so instead
  const split = !!mine && DAYS.some((d) => mine.shifts.filter((s) => s.weekday === d).length > 1);
  return (
    <>
      <p className="fine"><a href="/reviewer">← All requests</a></p>
      <div className="head"><div><h1>Review rota</h1><p className="lede">Who is working, in the desk&apos;s time ({offsetLabel(v.utc_offset_minutes)}; it is {v.local_now} now). It tells people who to expect; it never stops anyone from deciding.</p></div></div>
      <div className={`note ${v.on_duty_now.length ? "ok" : "warn"}`} role="status" style={{ marginBottom: 16 }}>
        {!v.configured ? "Nobody has set hours yet." : v.on_duty_now.length ? <>On duty now: <strong>{v.on_duty_now.join(", ")}</strong></> : <strong>Nobody is on duty now.</strong>}
      </div>
      <div className="tablewrap" style={{ marginBottom: 16 }}>
        <table>
          <thead><tr><th>Reviewer</th>{DAYS.map((d) => <th key={d} style={{ textTransform: "capitalize" }}>{d}</th>)}</tr></thead>
          <tbody>
            {v.reviewers.map((x) => (
              <tr key={x.id}>
                <td>{x.name}{x.me ? " (you)" : ""}{x.on_duty && <div className="fine">on duty now</div>}</td>
                {DAYS.map((d) => <td key={d} className="fine">{x.shifts.filter((s) => s.weekday === d).map((s) => `${s.start}–${s.end}`).join(", ") || "–"}</td>)}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <section className="card">
        <h2>Your hours</h2>
        {split
          ? <p className="fine">Your rota has more than one shift on a day, which this form cannot show. Change it with <code>scripts/platform-review.sh rota-set</code>.</p>
          : <><p className="fine">Tick the days you work. A shift that runs past midnight is two entries, so use <code>scripts/platform-review.sh rota-set</code> for those.</p><RotaForm shifts={mine?.shifts ?? []} /></>}
      </section>
    </>
  );
}
