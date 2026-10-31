export type Step = { key: string; label: string; done: boolean };

/** Numbered progress for a multi-step form. Each step is a link, so people can go back and check or fix an earlier one. */
export function Stepper({ steps, current, base }: { steps: Step[]; current: string; base: string }) {
  return (
    <ol className="stepper" aria-label="Progress">
      {steps.map((s, i) => (
        <li key={s.key} className={`${s.done ? "done" : ""} ${s.key === current ? "now" : ""}`}>
          <a href={`${base}?step=${s.key}`} aria-current={s.key === current ? "step" : undefined}>
            <span className="dot" aria-hidden="true">{s.done ? "✓" : i + 1}</span>
            <span className="lbl">{s.label}<span className="sr">{s.done ? " (done)" : ""}</span></span>
          </a>
        </li>
      ))}
    </ol>
  );
}
