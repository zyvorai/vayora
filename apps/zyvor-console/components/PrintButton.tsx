"use client";

export function PrintButton() {
  return <button className="btn small no-print" type="button" onClick={() => window.print()}>Print or save as PDF</button>;
}
