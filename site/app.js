(function () {
  "use strict";

  // nav hairline once you scroll
  var nav = document.querySelector(".nav");
  var onScroll = function () { nav.classList.toggle("scrolled", window.scrollY > 8); };
  onScroll();
  window.addEventListener("scroll", onScroll, { passive: true });

  // reveal chapters as they come into view (CSS shows everything when motion is reduced or JS is off)
  var els = document.querySelectorAll(".reveal");
  if ("IntersectionObserver" in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); } });
    }, { threshold: 0.18, rootMargin: "0px 0px -6% 0px" });
    els.forEach(function (el) { io.observe(el); });
  } else {
    els.forEach(function (el) { el.classList.add("in"); });
  }

  // the "Choose your setup" panel
  var SETUPS = {
    sandbox: {
      title: "Sandbox",
      sub: "Everything runs on your machine against a simulated provider. The fastest way to see a payment settle.",
      list: ["API, worker, checkout and console on your laptop", "Simulated Razorpay and Cashfree: no real money, no accounts", "A ready-to-open checkout URL and an API key", "Runnable examples: school fees, clinic billing, a shop counter, marketplace splits"],
      code: "git clone https://github.com/zyvorai/vayora.git\ncd vayora\n# needs a local Postgres and an EMPTY database\nOWNER_URL=postgres://you@localhost:5432/zyvor_sandbox \\\n  ./scripts/zyvor-sandbox.sh",
      cta: "Open the sandbox guide",
      href: "https://github.com/zyvorai/vayora#try-the-sandbox-simulated-providers-no-real-money"
    },
    self: {
      title: "Self-host",
      sub: "Run Zyvor on your own servers and connect your own provider accounts. Your provider collects and settles to your bank.",
      list: ["Apache-2.0: use it, change it, ship it", "Your own Razorpay, Cashfree or gateway (a small JSON contract)", "Kubernetes (k3s and Helm) and Compose assets included", "Licences are yours to hold or borrow: read the regulatory position first"],
      code: "git clone https://github.com/zyvorai/vayora.git\ncd vayora/deploy/zyvor/k3s\n# single-node k3s with HTTPS in front\n# follow README.md in this folder",
      cta: "Read the deploy guide",
      href: "https://github.com/zyvorai/vayora/blob/main/deploy/zyvor/k3s/README.md"
    },
    help: {
      title: "Talk to us",
      sub: "A question, a pilot, or help connecting a provider. There is no price list and Zyvor is not a hosted service today, so this is a conversation, not a checkout.",
      list: ["Tell us what you are building and where it runs", "We can help connect a provider or review your setup", "Honest answers, including when Zyvor is the wrong fit"],
      code: "",
      cta: "Contact us",
      href: "https://zyvor.dev/contact"
    }
  };

  var opts = Array.prototype.slice.call(document.querySelectorAll(".opt"));
  var $ = function (id) { return document.getElementById(id); };
  function show(key) {
    var s = SETUPS[key];
    opts.forEach(function (o) {
      var on = o.dataset.k === key;
      o.setAttribute("aria-checked", on ? "true" : "false");
      o.tabIndex = on ? 0 : -1;
    });
    $("s-title").textContent = s.title;
    $("s-sub").textContent = s.sub;
    var ul = $("s-list"); ul.textContent = "";
    s.list.forEach(function (t) { var li = document.createElement("li"); li.textContent = t; ul.appendChild(li); });
    $("s-code").textContent = s.code;
    $("s-codebox").hidden = !s.code;
    var cta = $("s-cta"); cta.textContent = s.cta; cta.href = s.href;
    $("copy").textContent = "Copy";
  }
  opts.forEach(function (o, i) {
    o.addEventListener("click", function () { show(o.dataset.k); });
    o.addEventListener("keydown", function (e) {
      var d = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
      if (!d) return;
      e.preventDefault();
      var n = opts[(i + d + opts.length) % opts.length];
      n.focus(); show(n.dataset.k);
    });
  });
  show("sandbox");

  // copy the commands
  $("copy").addEventListener("click", function () {
    var text = $("s-code").textContent, btn = this;
    var done = function (ok) { btn.textContent = ok ? "Copied" : "Press Ctrl+C"; setTimeout(function () { btn.textContent = "Copy"; }, 1800); };
    if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text).then(function () { done(true); }, function () { done(false); });
    else done(false);
  });

  var yr = $("yr"); if (yr) yr.textContent = new Date().getFullYear();
})();
