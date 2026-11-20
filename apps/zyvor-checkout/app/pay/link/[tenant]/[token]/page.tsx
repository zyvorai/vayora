// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { PayLink } from "@/components/PayLink";

export default async function Page({ params }: { params: Promise<{ tenant: string; token: string }> }) {
  const { tenant, token } = await params;
  return <PayLink tenant={tenant} token={token} />;
}
