import { NextResponse, type NextRequest } from "next/server";
import { toCsv } from "@/lib/csv";
import { COOKIE, readSession } from "@/lib/session";
import { api } from "@/lib/zyvor";

const MAX_ROWS = 10_000;

type Intent = { id: string; external_ref: string | null; amount_minor: number; currency?: string; status: string; created_at?: string };

// The proxy already requires a session; this re-checks so the route is safe if the matcher changes.
export async function GET(req: NextRequest, ctx: { params: Promise<{ kind: string }> }) {
  const s = await readSession(process.env.CONSOLE_SESSION_SECRET ?? "", req.cookies.get(COOKIE)?.value);
  if (!s) return new NextResponse("Sign in first", { status: 401 });
  const { kind } = await ctx.params;
  const status = req.nextUrl.searchParams.get("status") || undefined;
  const q = req.nextUrl.searchParams.get("q")?.trim() || undefined;
  const z = await api();
  let body: string;
  if (kind === "payments") {
    const rows: Intent[] = [];
    let after: string | undefined;
    while (rows.length < MAX_ROWS) {
      const page = await z.intents.list({ limit: 100, status, q, startingAfter: after });
      rows.push(...(page.data as unknown as Intent[]));
      const last = page.data.at(-1);
      if (!page.has_more || !last) break;
      after = last.id;
    }
    body = toCsv(["id", "reference", "amount_minor", "currency", "status", "created_at"], rows.map((i) => [i.id, i.external_ref, i.amount_minor, i.currency ?? "INR", i.status, i.created_at]));
  } else if (kind === "invoices") {
    const page = await z.invoices.list(status);
    body = toCsv(["id", "number", "amount_minor", "paid_minor", "outstanding_minor", "status", "due_date"], page.data.map((i) => [i.id, i.number, i.amount_minor, i.paid_minor, i.outstanding_minor, i.status, i.due_date]));
  } else {
    return new NextResponse("Unknown export", { status: 404 });
  }
  return new NextResponse(body, { headers: { "content-type": "text/csv; charset=utf-8", "content-disposition": `attachment; filename="zyvor-${kind}.csv"`, "cache-control": "no-store" } });
}
