export default function Loading() {
  return (
    <div aria-busy="true" aria-label="Loading">
      <div className="skel" style={{ height: 44, width: 260, marginBottom: 14 }} />
      <div className="skel" style={{ height: 20, width: 420, marginBottom: 32 }} />
      <div className="grid stats">{[0, 1, 2, 3].map((n) => <div key={n} className="skel" style={{ height: 120, borderRadius: 22 }} />)}</div>
      <div className="skel" style={{ height: 280, borderRadius: 22, marginTop: 8 }} />
    </div>
  );
}
