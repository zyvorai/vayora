import { EventPage } from "@/components/EventPage";

export default async function Page({ params }: { params: Promise<{ tenant: string; event: string }> }) {
  const { tenant, event } = await params;
  return <EventPage tenant={tenant} event={event} />;
}
