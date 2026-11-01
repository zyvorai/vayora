import { redirect } from "next/navigation";
import { DayOffForm, RemoveDayOff, RotaForm } from "@/components/ReviewerForms";
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
        {v.holiday_today ? <><strong>Desk holiday today ({v.holiday_today}).</strong> Nobody is on duty.</> : !v.configured ? "Nobody has set hours yet." : v.on_duty_now.length ? <>On duty now: <strong>{v.on_duty_now.join(", ")}</strong></> : <strong>Nobody is on duty now.</strong>}
      </div>
      <div className="tablewrap" style={{ marginBottom: 16 }}>
        <table>
          <thead><tr><th>Reviewer</th>{DAYS.map((d) => <th key={d} style={{ textTransform: "capitalize" }}>{d}</th>)}</tr></thead>
          <tbody>
            {v.reviewers.map((x) => (
              <tr key={x.id}>
                <td>{x.name}{x.me ? " (you)" : ""}{x.on_duty && <div className="fine">on duty now</div>}{x.on_leave && <div className="fine">on leave today</div>}</td>
                {DAYS.map((d) => <td key={d} className="fine">{x.shifts.filter((s) => s.weekday === d).map((s) => `${s.start}–${s.end}`).join(", ") || "–"}</td>)}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <section className="card" style={{ marginBottom: 16 }}>
        <h2>Leave and holidays</h2>
        {v.days_off.length === 0 ? <p className="fine">Nothing booked.</p> : (
          <div className="tablewrap" style={{ marginBottom: 12 }}>
            <table>
              <thead><tr><th>Days</th><th>Who</th><th>Note</th><th /></tr></thead>
              <tbody>
                {v.days_off.map((o) => (
                  <tr key={o.id}>
                    <td>{o.first_day === o.last_day ? o.first_day : `${o.first_day} to ${o.last_day}`}</td>
                    <td>{o.desk ? "Whole desk (holiday)" : `${o.reviewer}${o.mine ? " (you)" : ""}`}</td>
                    <td className="fine">{o.note || "–"}</td>
                    <td>{(o.desk || o.mine) && <RemoveDayOff id={o.id} />}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <p className="fine">Days are in the desk&apos;s time and include both ends. Someone on leave, and everyone on a holiday, is not on duty and is skipped by automatic assignment. Anything they already hold stays with them.</p>
        <DayOffForm today={v.local_date} />
      </section>
      <section className="card">
        <h2>Your hours</h2>
        {split
          ? <p className="fine">Your rota has more than one shift on a day, which this form cannot show. Change it with <code>scripts/platform-review.sh rota-set</code>.</p>
          : <><p className="fine">Tick the days you work. A shift that runs past midnight is two entries, so use <code>scripts/platform-review.sh rota-set</code> for those.</p><RotaForm shifts={mine?.shifts ?? []} /></>}
      </section>
    </>
  );
}
