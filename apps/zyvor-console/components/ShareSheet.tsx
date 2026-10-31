"use client";
import { useRef, useState } from "react";

/** Copy, QR code, WhatsApp and email for one payment link, in a dialog (a table's overflow would clip a popover). */
export function ShareSheet({ id, url, title }: { id: string; url: string; title: string }) {
  const dlg = useRef<HTMLDialogElement>(null);
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    } catch {
      window.prompt("Copy this link", url);
    }
  };
  const msg = encodeURIComponent(`${title}: ${url}`);
  return (
    <>
      <button type="button" className="btn small" aria-label={`Share ${title}`} onClick={() => { setOpen(true); dlg.current?.showModal(); }}>Share</button>
      <dialog ref={dlg} className="sharedlg" aria-label={`Share ${title}`} onClose={() => setOpen(false)} onClick={(e) => { if (e.target === dlg.current) dlg.current?.close(); }}>
        <strong>{title}</strong>
        {open && <img src={`/links/${encodeURIComponent(id)}/qr`} alt={`QR code for ${title}`} width="176" height="176" />}
        <div className="row">
          <button type="button" className="btn small primary" onClick={copy}>{copied ? "Copied" : "Copy link"}</button>
          <a className="btn small" href={`https://wa.me/?text=${msg}`} target="_blank" rel="noreferrer">WhatsApp</a>
          <a className="btn small" href={`mailto:?subject=${encodeURIComponent(title)}&body=${msg}`}>Email</a>
          <button type="button" className="btn small" onClick={() => dlg.current?.close()}>Close</button>
        </div>
        <span className="sr" role="status">{copied ? "Link copied" : ""}</span>
      </dialog>
    </>
  );
}
