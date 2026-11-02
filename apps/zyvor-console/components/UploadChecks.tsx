import type { OnboardingUpload } from "@zyvor/node";

const LABEL: Record<string, string> = {
  gstin_found: "GSTIN on the document",
  gstin_matches_profile: "GSTIN is the one entered",
  pan_matches_profile: "PAN ends the same as the one entered",
  legal_name_matches_profile: "Legal name is the one entered",
  cin_found: "Company identity number (CIN) on the document",
  ifsc_matches_account: "IFSC is that of an account entered",
  kept_for_the_reviewer: "Kept for the reviewer",
};
const WORDS: Record<string, [string, string]> = {
  match: ["ok", "matches"],
  mismatch: ["bad", "does not match"],
  not_found: ["warn", "not found"],
  not_checked: ["", "not compared"],
};
const HOW: Record<string, string> = { text: "read from the PDF's text", ocr: "read from the scan (OCR)", none: "could not be read" };

/** What was read from an uploaded document and whether it agrees with the business details: a word and a dot, never colour alone. */
export function UploadChecks({ u }: { u: Pick<OnboardingUpload, "read_method" | "facts" | "checks"> }) {
  return (
    <div>
      <div className="muted" style={{ fontSize: ".8125rem" }}>
        {u.read_method === "none" ? <span className="chip warn">{HOW.none}: a person will look at it</span> : HOW[u.read_method]}
      </div>
      <ul style={{ listStyle: "none", padding: 0, margin: "6px 0 0", display: "grid", gap: 4 }}>
        {u.checks.map((c) => {
          const [tone, word] = WORDS[c.result] ?? ["", c.result];
          return (
            <li key={c.name} style={{ display: "flex", gap: 8, alignItems: "baseline", flexWrap: "wrap" }}>
              <span className={`chip ${tone}`}>{word}</span>
              <span>{LABEL[c.name] ?? c.name}</span>
              {c.result !== "match" && c.result !== "not_checked" && <span className="muted">{c.detail}</span>}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
