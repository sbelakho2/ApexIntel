/* ─── ApexIntel App.js — Global Client Utilities ─────────────────────────── */
(function () {
  "use strict";

  /* ── Dark Mode ──────────────────────────────────────────────────────────── */
  const root = document.documentElement;
  function applyTheme(dark) {
    if (dark) { root.classList.add("dark"); root.setAttribute("data-theme", "dark"); }
    else { root.classList.remove("dark"); root.removeAttribute("data-theme"); }
  }
  function getPreferred() {
    const stored = localStorage.getItem("apex-theme");
    if (stored === "dark") return true;
    if (stored === "light") return false;
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  }
  applyTheme(getPreferred());
  window.toggleDarkMode = function () {
    const isDark = root.classList.contains("dark");
    const next = !isDark;
    localStorage.setItem("apex-theme", next ? "dark" : "light");
    applyTheme(next);
  };

  /* ── Toast Notifications ────────────────────────────────────────────────── */
  let toastContainer = null;
  function ensureToastContainer() {
    if (toastContainer && document.body.contains(toastContainer)) return toastContainer;
    toastContainer = document.createElement("div");
    toastContainer.id = "toast-container";
    toastContainer.className = "fixed bottom-4 right-4 z-50 flex flex-col gap-2";
    toastContainer.setAttribute("aria-live", "polite");
    document.body.appendChild(toastContainer);
    return toastContainer;
  }
  window.showToast = function (message, variant) {
    variant = variant || "info";
    const ct = ensureToastContainer();
    const el = document.createElement("div");
    const colors = {
      error: "border-destructive/30 bg-destructive/10 text-destructive",
      success: "border-green-500/30 bg-green-500/10 text-green-700",
      info: "border-border bg-card text-foreground",
      warning: "border-yellow-500/30 bg-yellow-500/10 text-yellow-700",
    };
    el.className = "flex items-center gap-2 rounded-md border px-3 py-2 text-sm shadow-lg " + (colors[variant] || colors.info);
    el.innerHTML = '<span class="max-w-xs text-xs">' + escapeHtml(message) + "</span>";
    ct.appendChild(el);
    setTimeout(function () { el.remove(); }, 4000);
  };

  function escapeHtml(s) {
    var d = document.createElement("div"); d.textContent = s; return d.innerHTML;
  }

  function getCookie(name) {
    var prefix = name + "=";
    var cookies = document.cookie ? document.cookie.split(";") : [];
    for (var index = 0; index < cookies.length; index += 1) {
      var cookie = cookies[index].trim();
      if (cookie.indexOf(prefix) === 0) {
        return decodeURIComponent(cookie.slice(prefix.length));
      }
    }
    return "";
  }

  function getCsrfToken() {
    return getCookie("apex_csrf");
  }

  function ensureCsrfField(form) {
    if (!form) return;
    var method = (form.getAttribute("method") || "get").toUpperCase();
    if (method === "GET" || method === "HEAD" || method === "OPTIONS") return;
    var token = getCsrfToken();
    if (!token) return;
    var input = form.querySelector('input[name="csrf_token"]');
    if (!input) {
      input = document.createElement("input");
      input.type = "hidden";
      input.name = "csrf_token";
      form.appendChild(input);
    }
    input.value = token;
  }

  function applyCsrfToForms(rootNode) {
    var rootNodeOrDocument = rootNode || document;
    var forms = rootNodeOrDocument.querySelectorAll ? rootNodeOrDocument.querySelectorAll("form") : [];
    forms.forEach(ensureCsrfField);
  }

  window.apexCsrfToken = getCsrfToken;

  /* ── WebSocket Warning Listener ─────────────────────────────────────────── */
  var wsRetryDelay = 1000;
  function connectWarningsWs() {
    var proto = location.protocol === "https:" ? "wss:" : "ws:";
    var ws = new WebSocket(proto + "//" + location.host + "/ws/warnings");
    ws.onopen = function () { wsRetryDelay = 1000; };
    ws.onmessage = function (evt) {
      try {
        var data = JSON.parse(evt.data);
        showToast(data.title || "New warning detected", "warning");
        // Update badge count
        var badge = document.getElementById("warning-badge");
        if (badge) {
          var current = parseInt(badge.textContent, 10) || 0;
          badge.textContent = current + 1;
          badge.classList.remove("hidden");
        }
      } catch (e) { /* ignore malformed messages */ }
    };
    ws.onclose = function () {
      setTimeout(function () {
        wsRetryDelay = Math.min(wsRetryDelay * 2, 30000);
        connectWarningsWs();
      }, wsRetryDelay);
    };
    ws.onerror = function () { ws.close(); };
  }
  if (document.querySelector("[data-ws-warnings]")) {
    connectWarningsWs();
  }

  /* ── Keyboard Shortcuts ─────────────────────────────────────────────────── */
  var shortcuts = {
    "1": "/", "2": "/warnings", "3": "/insights", "4": "/memos",
    "5": "/companies", "6": "/persons", "7": "/security", "8": "/graph",
    "9": "/recipes", "0": "/settings",
  };
  document.addEventListener("keydown", function (e) {
    if (!e.altKey || e.metaKey || e.ctrlKey || e.shiftKey) return;
    var target = shortcuts[e.key];
    if (target) { e.preventDefault(); window.location.href = target; }
  });

  /* ── Offline Detection ──────────────────────────────────────────────────── */
  function updateOnlineStatus() {
    var banner = document.getElementById("offline-banner");
    if (!banner) return;
    if (navigator.onLine) { banner.classList.add("hidden"); }
    else { banner.classList.remove("hidden"); }
  }
  window.addEventListener("online", updateOnlineStatus);
  window.addEventListener("offline", updateOnlineStatus);
  document.addEventListener("DOMContentLoaded", updateOnlineStatus);
  document.addEventListener("DOMContentLoaded", function () {
    applyCsrfToForms(document);
  });

  document.addEventListener("submit", function (evt) {
    ensureCsrfField(evt.target);
  }, true);

  /* ── Dense Table Navigation ────────────────────────────────────────────── */
  document.addEventListener("click", function (evt) {
    var row = evt.target.closest ? evt.target.closest("[data-row-link]") : null;
    if (!row) return;
    if (evt.target.closest("a, button, input, select, textarea")) return;
    var href = row.getAttribute("data-row-link");
    if (href) { window.location.href = href; }
  });

  document.addEventListener("keydown", function (evt) {
    var row = evt.target.closest ? evt.target.closest("[data-row-link]") : null;
    if (!row) return;
    if (evt.key === "Enter") {
      evt.preventDefault();
      var href = row.getAttribute("data-row-link");
      if (href) window.location.href = href;
      return;
    }
    if (evt.key !== "ArrowDown" && evt.key !== "ArrowUp") return;
    var table = row.closest("[data-dense-table]");
    if (!table) return;
    var rows = Array.prototype.slice.call(table.querySelectorAll("tbody [data-row-link]"));
    var index = rows.indexOf(row);
    if (index === -1) return;
    evt.preventDefault();
    var delta = evt.key === "ArrowDown" ? 1 : -1;
    var next = rows[index + delta];
    if (next) {
      next.focus();
      next.scrollIntoView({ block: "nearest" });
    }
  });

  /* ── CSV Export ─────────────────────────────────────────────────────────── */
  window.exportCSV = function (url, filename) {
    fetch(url, { credentials: "include" })
      .then(function (r) { return r.json(); })
      .then(function (json) {
        var data = json.data || json;
        var items = data.items || data;
        if (!Array.isArray(items) || items.length === 0) {
          showToast("No data to export", "info"); return;
        }
        var headers = Object.keys(items[0]);
        var rows = [headers.join(",")];
        items.forEach(function (item) {
          var row = headers.map(function (h) {
            var v = item[h];
            if (v == null) return "";
            var s = String(v);
            if (s.indexOf(",") !== -1 || s.indexOf('"') !== -1 || s.indexOf("\n") !== -1) {
              return '"' + s.replace(/"/g, '""') + '"';
            }
            return s;
          });
          rows.push(row.join(","));
        });
        var blob = new Blob([rows.join("\n")], { type: "text/csv;charset=utf-8;" });
        var link = document.createElement("a");
        link.href = URL.createObjectURL(blob);
        link.download = filename || "export.csv";
        link.click();
        showToast("CSV exported", "success");
      })
      .catch(function (err) { showToast("Export failed: " + err.message, "error"); });
  };

  /* ── Clipboard Copy ─────────────────────────────────────────────────────── */
  window.copyId = function (text, el) {
    navigator.clipboard.writeText(text).then(function () {
      if (el) {
        var orig = el.textContent;
        el.textContent = "Copied!";
        setTimeout(function () { el.textContent = orig; }, 1500);
      }
      showToast("Copied to clipboard", "success");
    }).catch(function () {
      showToast("Copy failed", "error");
    });
  };

  /* ── HTMX Event Hooks ──────────────────────────────────────────────────── */
  document.addEventListener("htmx:configRequest", function (evt) {
    var verb = String(evt.detail.verb || "GET").toUpperCase();
    if (verb === "GET" || verb === "HEAD" || verb === "OPTIONS") return;
    var token = getCsrfToken();
    if (!token) return;
    evt.detail.headers["X-CSRF-Token"] = token;
  });

  document.addEventListener("htmx:afterRequest", function (evt) {
    if (evt.detail.failed) {
      showToast("Request failed: " + (evt.detail.xhr.status || "network error"), "error");
    }
  });

  document.addEventListener("htmx:afterSwap", function (evt) {
    applyCsrfToForms(evt.target || document);
  });

  /* ── Route progress bar ─────────────────────────────────────────────────── */
  document.addEventListener("htmx:beforeRequest", function () {
    var bar = document.getElementById("route-progress");
    if (bar) bar.classList.add("is-active");
  });
  document.addEventListener("htmx:afterSettle", function () {
    var bar = document.getElementById("route-progress");
    if (bar) bar.classList.remove("is-active");
  });
})();
