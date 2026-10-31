/** The visitor's address from a proxy's `X-Forwarded-For`, trusting `hops` proxies in front of the console.
 *  With 0 hops nothing is trusted (a caller could write anything there), so there is no address. */
export function visitorIp(xff: string | null | undefined, hops: number): string | null {
  if (!xff || !Number.isInteger(hops) || hops < 1) return null;
  const parts = xff.split(",").map((p) => p.trim()).filter(Boolean);
  const ip = parts[parts.length - hops];
  if (!ip) return null;
  // IPv4 or IPv6 characters only; the API parses it again and ignores anything else
  return /^[0-9a-fA-F:.]{2,45}$/.test(ip) && (ip.includes(".") || ip.includes(":")) ? ip : null;
}
