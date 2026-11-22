// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import "server-only";
import { redirect } from "next/navigation";
import { apiAccess, sessionKey } from "@/lib/zyvor";
import { callJson, type ApiResult } from "@/lib/restcall";

/** The API for the endpoints the SDK has no method for yet (fee checks, bank statements, routing health). Same key, same
 *  signed actor as `api()`; for a person signed in by email, a key the API no longer accepts sends them back to sign in. */
export async function rest(actor?: string) {
  const { key, base } = await apiAccess();
  const organisation = !!(await sessionKey());
  const go = async <T>(method: "GET" | "POST" | "PUT" | "DELETE", path: string, body?: unknown): Promise<ApiResult<T>> => {
    const r = await callJson<T>({ base, key, actor: actor ?? process.env.CONSOLE_USER ?? "console", actorSecret: process.env.ZYVOR_ACTOR_SECRET || undefined }, method, path, body);
    if (!r.ok && r.status === 401 && organisation) redirect("/login");
    return r;
  };
  return {
    get: <T>(path: string) => go<T>("GET", path),
    post: <T>(path: string, body?: unknown) => go<T>("POST", path, body),
    put: <T>(path: string, body: unknown) => go<T>("PUT", path, body),
    del: <T>(path: string) => go<T>("DELETE", path),
  };
}
