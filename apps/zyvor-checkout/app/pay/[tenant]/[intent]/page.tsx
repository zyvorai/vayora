// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { Suspense } from "react";
import { Checkout } from "@/components/Checkout";

export default async function Page({ params }: { params: Promise<{ tenant: string; intent: string }> }) {
  const { tenant, intent } = await params;
  return (
    <Suspense fallback={<div className="card state" role="status"><div className="spin" /><p>Loading…</p></div>}>
      <Checkout tenant={tenant} intent={intent} />
    </Suspense>
  );
}
