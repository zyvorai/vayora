// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

"use client";

export function PrintButton() {
  return <button className="btn small no-print" type="button" onClick={() => window.print()}>Print or save as PDF</button>;
}
