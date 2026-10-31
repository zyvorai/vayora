import Link from "next/link";

export default function NotFound() {
  return (
    <div className="card" style={{ maxWidth: 560 }}>
      <h1 style={{ fontSize: "1.5rem" }}>We could not find that</h1>
      <p className="lede">It may have been removed, or it belongs to the other environment (Test and Live are separate).</p>
      <p><Link className="btn primary" href="/">Back to overview</Link></p>
    </div>
  );
}
