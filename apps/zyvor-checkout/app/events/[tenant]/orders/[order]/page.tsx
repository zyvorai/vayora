import { Suspense } from "react";
import { OrderPage } from "@/components/OrderPage";

export default async function Page({ params }: { params: Promise<{ tenant: string; order: string }> }) {
  const { tenant, order } = await params;
  return (
    <Suspense fallback={<div className="card state" role="status"><div className="spin" /><p>Loading…</p></div>}>
      <OrderPage tenant={tenant} order={order} />
    </Suspense>
  );
}
