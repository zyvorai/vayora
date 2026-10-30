(() => {
  // ---- theme toggle ----
  const root = document.documentElement;
  document.getElementById("theme").addEventListener("click", () => {
    const next = root.dataset.theme === "dark" ? "light" : "dark";
    root.dataset.theme = next;
    try { localStorage.setItem("zyvor-theme", next); } catch {}
  });

  // ---- hero cell stream (decorative, simulated) ----
  const grid = document.getElementById("cells");
  const ROWS = 6, COLS = 36;
  grid.style.setProperty("--cols", COLS);
  const cells = Array.from({ length: ROWS * COLS }, () => grid.appendChild(document.createElement("i")));
  // fill shaped like a decaying curve, like the reference's utilisation grid
  const fill = [0.2, 0.14, 0.11, 0.06, 0, 0].map((f) => Math.round(f * COLS));
  const target = cells.map((_, i) => (i % COLS) < fill[Math.floor(i / COLS)]);
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) cells.forEach((c, i) => c.classList.toggle("on", target[i]));
  else {
    let n = 0;
    setInterval(() => {
      const i = n++ % (ROWS * COLS);
      if (i === 0) cells.forEach((c) => c.classList.remove("on"));
      // light the shaped cells in reading order, plus an occasional extra spark
      if (target[i]) cells[i].classList.add("on");
      const s = Math.floor(Math.random() * cells.length);
      if (target[s]) cells[s].classList.add("on");
    }, 60);
  }

  const $ = (id) => document.getElementById(id);
  const inr = (p) => "₹" + (p / 100).toLocaleString("en-IN", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  const AMOUNT = 49900, REFUND = 10000;
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const sleep = (ms) => new Promise((r) => setTimeout(r, reduce ? Math.min(ms, 80) : ms));

  // ---- demo: mirrors the Rust SimulatedUpi (fail* declines, timeout* times out, else success) ----
  const vpaOk = (v) => /^[A-Za-z0-9._-]{2,64}@[A-Za-z]{2,32}$/.test(v);
  const outcome = (v) => {
    const h = v.split("@")[0];
    return h.startsWith("fail") ? { ok: false, reason: "insufficient_funds" }
      : h.startsWith("timeout") ? { ok: false, reason: "payer_timeout" }
      : { ok: true };
  };
  const rid = (p) => p + "_" + Array.from(crypto.getRandomValues(new Uint8Array(8)), (b) => b.toString(16).padStart(2, "0")).join("");
  const rrn = () => String(Math.floor(Math.random() * 1e12)).padStart(12, "0");

  let payable = 0, payment = null, refunded = 0;

  async function sign(secret, t, body) {
    try {
      const enc = new TextEncoder();
      const key = await crypto.subtle.importKey("raw", enc.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
      const mac = await crypto.subtle.sign("HMAC", key, enc.encode(`${t}.${body}`));
      return Array.from(new Uint8Array(mac), (b) => b.toString(16).padStart(2, "0")).join("");
    } catch { return "(needs a secure context)"; }
  }
  async function emit(type, data) {
    const body = JSON.stringify({ id: rid("evt"), type, created_at: new Date().toISOString(), data });
    const t = Math.floor(Date.now() / 1000);
    $("hook").textContent = `Zyvor-Signature: t=${t},v1=${await sign("whsec_demo", t, body)}\n\n` + JSON.stringify(JSON.parse(body), null, 2);
  }
  function addRows(rows) {
    const tb = $("ledger").tBodies[0];
    if (tb.querySelector(".empty")) tb.innerHTML = "";
    for (const [side, acct, amt] of rows) {
      const tr = document.createElement("tr");
      tr.className = "in";
      tr.innerHTML = `<td><span class="tag ${side === "Debit" ? "dr" : "cr"}">${side}</span><code></code></td><td></td>`;
      tr.querySelector("code").textContent = acct;
      tr.lastChild.textContent = inr(amt);
      tb.appendChild(tr);
    }
    $("bal").textContent = inr(payable);
  }
  function show(kind, title, text) {
    const r = $("result");
    r.className = "result show " + kind;
    $("mark").setAttribute("d", kind === "ok" ? "M26 43l11 11 21-23" : "M30 30l24 24M54 30L30 54");
    $("rtitle").textContent = title;
    $("rtext").textContent = text;
    $("refund").hidden = !(kind === "ok" && payment && refunded < payment.amount);
    (kind === "ok" && !$("refund").hidden ? $("refund") : $("again")).focus({ preventScroll: true });
  }

  $("pay").addEventListener("click", async () => {
    const v = $("vpa").value.trim();
    if (!vpaOk(v)) { $("vpa").setAttribute("aria-invalid", "true"); $("vpa").focus(); return; }
    $("vpa").removeAttribute("aria-invalid");
    const b = $("pay"); b.disabled = true; b.innerHTML = '<span class="spin" aria-hidden="true"></span> Waiting for approval…';
    await sleep(1300);
    const o = outcome(v);
    const id = rid("pay");
    if (o.ok) {
      payment = { id, amount: AMOUNT, upi_ref: rrn() }; refunded = 0; payable += AMOUNT;
      addRows([["Debit", "upi_clearing", AMOUNT], ["Credit", "merchant_payable", AMOUNT]]);
      show("ok", "Payment successful", `${inr(AMOUNT)} paid · UPI ref ${payment.upi_ref}`);
      emit("payment.captured", { payment: { id, order_id: "order_demo", vpa: v, amount_paise: AMOUNT, status: "captured", upi_ref: payment.upi_ref, failure_reason: null } });
    } else {
      show("bad", "Payment declined", o.reason === "payer_timeout" ? "The payer didn’t approve in time. Nothing was charged." : "Insufficient funds. Nothing was charged.");
      emit("payment.failed", { payment: { id, order_id: "order_demo", vpa: v, amount_paise: AMOUNT, status: "failed", upi_ref: null, failure_reason: o.reason } });
    }
  });

  $("refund").addEventListener("click", () => {
    refunded += REFUND; payable -= REFUND;
    addRows([["Debit", "merchant_payable", REFUND], ["Credit", "upi_clearing", REFUND]]);
    emit("refund.processed", { refund: { id: rid("rfnd"), payment_id: payment.id, amount_paise: REFUND, status: "processed" } });
    show("ok", "Refund processed", `${inr(REFUND)} returned · ${inr(payment.amount - refunded)} left to refund`);
  });

  $("again").addEventListener("click", () => {
    $("result").className = "result";
    const b = $("pay"); b.disabled = false; b.textContent = "Pay with UPI"; b.focus({ preventScroll: true });
  });
  document.querySelectorAll("[data-vpa]").forEach((el) => el.addEventListener("click", () => { $("vpa").value = el.dataset.vpa; $("vpa").focus(); }));
  $("vpa").addEventListener("keydown", (e) => { if (e.key === "Enter") $("pay").click(); });

  // ---- code tabs (WAI-ARIA tabs pattern) ----
  const tabs = [...document.querySelectorAll('[role="tab"]')];
  const select = (t) => tabs.forEach((x) => {
    const on = x === t;
    x.setAttribute("aria-selected", on); x.tabIndex = on ? 0 : -1;
    document.getElementById(x.getAttribute("aria-controls")).hidden = !on;
  });
  tabs.forEach((t, i) => {
    t.addEventListener("click", () => select(t));
    t.addEventListener("keydown", (e) => {
      const n = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
      if (n) { const nt = tabs[(i + n + tabs.length) % tabs.length]; select(nt); nt.focus(); e.preventDefault(); }
    });
  });
  $("copy").addEventListener("click", async () => {
    const pane = document.querySelector('[role="tabpanel"]:not([hidden])');
    try { await navigator.clipboard.writeText(pane.innerText); $("copy").textContent = "Copied"; } catch { $("copy").textContent = "Press ⌘C"; }
    setTimeout(() => ($("copy").textContent = "Copy"), 1500);
  });

  // ---- scroll reveal ----
  const items = document.querySelectorAll(".reveal");
  if ("IntersectionObserver" in window && !reduce) {
    const io = new IntersectionObserver((es) => es.forEach((e) => { if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); } }), { threshold: 0.15 });
    items.forEach((el, i) => { el.style.transitionDelay = (i % 3) * 80 + "ms"; io.observe(el); });
  } else items.forEach((el) => el.classList.add("in"));
})();
