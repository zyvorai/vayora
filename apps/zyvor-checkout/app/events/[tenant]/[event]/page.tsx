// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { EventPage } from "@/components/EventPage";

export default async function Page({ params }: { params: Promise<{ tenant: string; event: string }> }) {
  const { tenant, event } = await params;
  return <EventPage tenant={tenant} event={event} />;
}
