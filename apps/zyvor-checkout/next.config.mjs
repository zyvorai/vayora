// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

const api = process.env.ZYVOR_API_URL ?? "http://localhost:8080";
export default {
  agentRules: false,
  // The browser talks to this origin only; the checkout API is proxied. No CORS, no API URL in the page.
  async rewrites() {
    return [
      { source: "/api/zyvor/:path*", destination: `${api}/v1/checkout/:path*` },
      // event pages: the public event, order and ticket endpoints
      { source: "/api/zyvor-events/:path*", destination: `${api}/v1/public/events/:path*` },
      { source: "/api/store/:path*", destination: `${api}/v1/store/:path*` },
    ];
  },
  async headers() {
    return [{ source: "/:path*", headers: [{ key: "Referrer-Policy", value: "no-referrer" }, { key: "X-Content-Type-Options", value: "nosniff" }, { key: "X-Frame-Options", value: "DENY" }] }];
  },
};
