import { PayLink } from "@/components/PayLink";

export default async function Page({ params }: { params: Promise<{ tenant: string; token: string }> }) {
  const { tenant, token } = await params;
  return <PayLink tenant={tenant} token={token} />;
}
