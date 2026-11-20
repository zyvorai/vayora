// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Checkout page weight and load time, measured against a locally served production build of apps/zyvor-checkout.
// Two independent measurements, both local:
//   1. curl accounting: the bytes the server actually sends (compressed) for the HTML and every /_next static file it references
//   2. a headless browser (Chrome, if found) through the DevTools protocol: every request the page makes, with and without
//      a throttled-network profile. Skipped (and said so) when no Chrome is available.
// Usage: node checkout_weight.mjs <checkout_base_url> <out.json>
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { newTenant, setRoutingFees, ok, writeJson, sleep, round } from "./common.mjs";

const base = process.argv[2];
const t = await newTenant("checkout weight");
await setRoutingFees(t);
const intent = await ok(t.key, "POST", "/v1/payment_intents", { amount_minor: 249_900, currency: "INR", description: "Benchmark order" }, { idem: "bench-weight-" + Date.now() });
const url = `${base}/pay/${t.id}/${intent.id}?s=${intent.client_secret}`;

function curl(u, extra = []) {
  const out = execFileSync("curl", ["-s", "-o", "-", "-H", "Accept-Encoding: gzip, br", "-w", "\n%{size_download}|%{size_header}|%{content_type}|%{http_code}", ...extra, u], { maxBuffer: 64 * 1024 * 1024 });
  const nl = out.lastIndexOf(10);
  const [dl, hdr, ctype, code] = out.subarray(nl + 1).toString().split("|");
  const raw = execFileSync("curl", ["-s", "--compressed", "-H", "Accept-Encoding: gzip, br", u], { maxBuffer: 64 * 1024 * 1024 });
  return { transferred: Number(dl), headers: Number(hdr), ctype, code: Number(code), decoded: raw.length, body: raw.toString("utf8") };
}

const html = curl(url);
const refs = [...new Set([...html.body.matchAll(/\/_next\/static\/[^"'\\\s)<>]+/g)].map((m) => m[0]))];
const kind = (u, ct) => (/\.js(\?|$)/.test(u) ? "js" : /\.css(\?|$)/.test(u) ? "css" : /\.(woff2?|ttf|otf)(\?|$)/.test(u) ? "font" : ct?.includes("html") ? "html" : "other");
const files = [{ url: url.replace(base, ""), kind: "html", transferred: html.transferred, decoded: html.decoded }];
for (const r of refs) {
  const x = curl(base + r);
  if (x.code === 200) files.push({ url: r, kind: kind(r, x.ctype), transferred: x.transferred, decoded: x.decoded });
}
const sum = (k, f) => files.filter((x) => !k || x.kind === k).reduce((s, x) => s + x[f], 0);
const curlResult = {
  note: "bytes the server sends for the HTML and every static file it references, with gzip/br accepted; excludes API calls the page makes after it loads",
  html_js_css_font_transferred_bytes: sum(null, "transferred"),
  html_js_css_font_decoded_bytes: sum(null, "decoded"),
  by_kind_transferred_bytes: Object.fromEntries(["html", "js", "css", "font", "other"].map((k) => [k, sum(k, "transferred")])),
  files: files.length,
};

// ---- headless browser through the DevTools protocol ----
function findChrome() {
  const c = [process.env.BENCH_CHROME, "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"].filter(Boolean);
  return c.find((p) => existsSync(p));
}
async function measureInBrowser(chrome, profile) {
  const dir = mkdtempSync(join(tmpdir(), "vayora-chrome-"));
  const proc = spawn(chrome, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${dir}`, "--disable-gpu", "--no-first-run", "--no-default-browser-check", "--disable-extensions", "about:blank"], { stdio: "ignore" });
  try {
    let ws = null;
    for (let i = 0; i < 100 && !ws; i++) {
      await sleep(100);
      try {
        const [port, path] = readFileSync(join(dir, "DevToolsActivePort"), "utf8").trim().split("\n");
        ws = `ws://127.0.0.1:${port}${path}`;
      } catch { /* not ready */ }
    }
    if (!ws) return { skipped: "Chrome did not start" };
    const sock = new WebSocket(ws);
    await new Promise((res, rej) => { sock.onopen = res; sock.onerror = rej; });
    let id = 0;
    const pending = new Map();
    const reqs = new Map();
    let lastActivity = Date.now(), loaded = null, sessionId = null;
    const start = { v: 0 };
    sock.onmessage = (m) => {
      const d = JSON.parse(m.data);
      if (d.id && pending.has(d.id)) { pending.get(d.id)(d.result ?? d.error); pending.delete(d.id); return; }
      lastActivity = Date.now();
      if (d.method === "Network.requestWillBeSent") reqs.set(d.params.requestId, { url: d.params.request.url, type: d.params.type, bytes: 0 });
      if (d.method === "Network.responseReceived") { const r = reqs.get(d.params.requestId); if (r) { r.status = d.params.response.status; r.mime = d.params.response.mimeType; } }
      if (d.method === "Network.loadingFinished") { const r = reqs.get(d.params.requestId); if (r) { r.bytes = d.params.encodedDataLength; r.done = true; } }
      if (d.method === "Page.loadEventFired") loaded = performance.now() - start.v;
    };
    const send = (method, params = {}, withSession = true) => new Promise((res) => { const i = ++id; pending.set(i, res); sock.send(JSON.stringify({ id: i, method, params, ...(withSession && sessionId ? { sessionId } : {}) })); });
    const { targetId } = await send("Target.createTarget", { url: "about:blank" }, false);
    ({ sessionId } = await send("Target.attachToTarget", { targetId, flatten: true }, false));
    await send("Network.enable");
    await send("Page.enable");
    await send("Network.setCacheDisabled", { cacheDisabled: true });
    if (profile) await send("Network.emulateNetworkConditions", { offline: false, latency: profile.latency_ms, downloadThroughput: profile.down_bytes_per_s, uploadThroughput: profile.up_bytes_per_s });
    start.v = performance.now();
    await send("Page.navigate", { url });
    // done when the load event has fired and nothing has happened on the network for 1.5 s
    const deadline = Date.now() + 90_000;
    while (Date.now() < deadline && !(loaded !== null && Date.now() - lastActivity > 1500)) await sleep(100);
    const list = [...reqs.values()].filter((r) => r.done);
    const by = {};
    for (const r of list) by[r.type ?? "Other"] = (by[r.type ?? "Other"] ?? 0) + r.bytes;
    sock.close();
    return {
      requests: list.length,
      transferred_bytes_total: list.reduce((s, r) => s + r.bytes, 0),
      by_resource_type_bytes: by,
      load_event_ms: loaded === null ? null : Math.round(loaded),
      failed_or_unfinished_requests: reqs.size - list.length,
    };
  } finally {
    proc.kill();
    await sleep(300);
    rmSync(dir, { recursive: true, force: true });
  }
}

const chrome = findChrome();
const browser = { chrome_found: Boolean(chrome) };
if (chrome) {
  browser.version = execFileSync(chrome, ["--version"], { encoding: "utf8" }).trim();
  browser.unthrottled = await measureInBrowser(chrome, null);
  // Chrome DevTools' "Fast 3G" preset values: 1.6 Mbit/s down, 750 kbit/s up, 562.5 ms round trip
  const profile = { name: "fast-3g-like", latency_ms: 562.5, down_bytes_per_s: Math.round((1.6e6 * 0.9) / 8), up_bytes_per_s: Math.round((750e3 * 0.9) / 8) };
  browser.throttled_profile = profile;
  browser.throttled = await measureInBrowser(chrome, profile);
} else browser.note = "no Chrome/Chromium found (set BENCH_CHROME); only the curl accounting was done";

await writeJson(process.argv[3], { description: "Checkout page weight (local production build served by `next start`; API on the same machine)", page: "/pay/{tenant}/{intent}", curl_accounting: curlResult, headless_browser: browser, file_list: files });
console.log(`checkout weight: ${curlResult.html_js_css_font_transferred_bytes} bytes transferred (curl)`);
