import type { Metadata } from "next";
import { Storefront } from "@/components/Storefront";

const api = process.env.ZYVOR_API_URL ?? "http://localhost:8080";

/** The page asks search engines to stay away unless the seller turned that off. Titles are escaped by Next. */
export async function generateMetadata({ params }: { params: Promise<{ slug: string }> }): Promise<Metadata> {
  const { slug } = await params;
  try {
    const r = await fetch(`${api}/v1/store/${encodeURIComponent(slug)}`, { cache: "no-store" });
    if (r.ok) {
      const j = (await r.json()) as { title: string; description: string; noindex: boolean };
      return { title: j.title, description: j.description.slice(0, 160) || undefined, robots: j.noindex ? "noindex, nofollow" : "index, follow" };
    }
  } catch {
    // fall through to the closed default
  }
  return { title: "Shop", robots: "noindex, nofollow" };
}

export default async function Page({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;
  return <Storefront slug={slug} />;
}
