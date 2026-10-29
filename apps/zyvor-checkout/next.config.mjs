const api = process.env.ZYVOR_API_URL ?? "http://localhost:8080";
export default {
  agentRules: false,
  // The browser talks to this origin only; the checkout API is proxied. No CORS, no API URL in the page.
  async rewrites() {
    return [{ source: "/api/zyvor/:path*", destination: `${api}/v1/checkout/:path*` }];
  },
  async headers() {
    return [{ source: "/:path*", headers: [{ key: "Referrer-Policy", value: "no-referrer" }, { key: "X-Content-Type-Options", value: "nosniff" }, { key: "X-Frame-Options", value: "DENY" }] }];
  },
};
