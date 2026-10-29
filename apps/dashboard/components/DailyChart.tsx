"use client";
import { useState } from "react";
import { inr, inrCompact, istDay } from "@/lib/format";

type Day = { date: string; gross_paise: number; count: number };

// Round the axis max up to 1/2/4/8 x 10^n so four equal steps land on round numbers.
function niceMax(v: number): number {
  if (v <= 0) return 100_00;
  const mag = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 4, 8, 10]) if (v <= m * mag) return m * mag;
  return 10 * mag;
}

const W = 640, H = 220, L = 46, R = 8, T = 10, B = 24;

export function DailyChart({ days }: { days: Day[] }) {
  const [hover, setHover] = useState<number | null>(null);
  const [table, setTable] = useState(false);
  const max = niceMax(Math.max(...days.map((d) => d.gross_paise)));
  const ticks = [0, 1, 2, 3, 4].map((i) => (max / 4) * i);
  const slot = (W - L - R) / days.length;
  const bw = Math.min(26, slot * 0.62);
  const y = (v: number) => T + (H - T - B) * (1 - v / max);
  const total = days.reduce((s, d) => s + d.gross_paise, 0);

  return (
    <div>
      <div className="chart-actions">
        <span className="muted" style={{ fontSize: ".8125rem" }}>Captured volume per day, last 14 days (IST)</span>
        <button className="linkbtn" type="button" onClick={() => setTable((t) => !t)} aria-pressed={table}>
          {table ? "View chart" : "View as table"}
        </button>
      </div>
      {table ? (
        <div className="tablewrap" style={{ boxShadow: "none" }}>
          <table>
            <thead><tr><th>Day</th><th className="num">Payments</th><th className="num">Volume</th></tr></thead>
            <tbody>{days.map((d) => (<tr key={d.date}><td>{istDay(d.date)}</td><td className="num">{d.count}</td><td className="num">{inr(d.gross_paise)}</td></tr>))}</tbody>
          </table>
        </div>
      ) : total === 0 ? (
        <div className="empty">No captured payments in the last 14 days.</div>
      ) : (
        <div className="chartwrap chart">
          <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`Bar chart of daily captured volume. Total ${inr(total)} over 14 days. Use View as table for exact values.`}>
            <g className="grid">
              {ticks.map((t) => (
                <g key={t}>
                  <line x1={L} x2={W - R} y1={y(t)} y2={y(t)} />
                  <text x={L - 8} y={y(t) + 4} textAnchor="end">{inrCompact(t)}</text>
                </g>
              ))}
            </g>
            <g className="bars">
              {days.map((d, i) => {
                const x = L + slot * i + (slot - bw) / 2;
                const h = Math.max(d.gross_paise > 0 ? 3 : 0, H - B - y(d.gross_paise));
                return (
                  <g key={d.date}>
                    {d.gross_paise > 0 && (
                      // 4px rounded data end, square on the baseline
                      <path className="bar" d={`M${x},${H - B} v${-(h - 4)} a4,4 0 0 1 4,-4 h${bw - 8} a4,4 0 0 1 4,4 v${h - 4} z`} />
                    )}
                    <rect className="hit" x={L + slot * i} y={T} width={slot} height={H - T - B}
                      onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)} />
                    {(i % 2 === 0 || days.length < 8) && <text x={x + bw / 2} y={H - 6} textAnchor="middle">{istDay(d.date)}</text>}
                  </g>
                );
              })}
            </g>
          </svg>
          {hover !== null && (
            <div className="tip" style={{ left: `${((L + slot * hover + slot / 2) / W) * 100}%`, top: `${(Math.min(y(days[hover].gross_paise), H - B - 8) / H) * 100}%` }}>
              <b>{inr(days[hover].gross_paise)}</b>
              {istDay(days[hover].date)} · {days[hover].count} payment{days[hover].count === 1 ? "" : "s"}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
