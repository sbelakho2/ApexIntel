(function () {
  'use strict';

  var routeProgress = document.getElementById('route-progress');

  function setRouteProgress(active) {
    if (!routeProgress) {
      return;
    }
    routeProgress.classList.toggle('is-active', active);
  }

  function getCookie(name) {
    var prefix = name + '=';
    var cookies = document.cookie ? document.cookie.split(';') : [];
    for (var index = 0; index < cookies.length; index += 1) {
      var cookie = cookies[index].trim();
      if (cookie.indexOf(prefix) === 0) {
        try {
          return decodeURIComponent(cookie.slice(prefix.length));
        } catch (error) {
          // A malformed cookie value must never take down the whole page's
          // scripts; fall back to the raw value.
          return cookie.slice(prefix.length);
        }
      }
    }
    return '';
  }

  function getCsrfToken() {
    // COOKIE_SECURE deployments use the __Host- prefixed cookie name.
    return getCookie('apex_csrf') || getCookie('__Host-apex_csrf');
  }

  function ensureCsrfField(form) {
    if (!(form instanceof HTMLFormElement)) {
      return;
    }
    var method = (form.getAttribute('method') || 'get').toUpperCase();
    if (method === 'GET' || method === 'HEAD' || method === 'OPTIONS') {
      return;
    }
    var token = getCsrfToken();
    if (!token) {
      return;
    }
    var input = form.querySelector('input[name="csrf_token"]');
    if (!input) {
      input = document.createElement('input');
      input.type = 'hidden';
      input.name = 'csrf_token';
      form.appendChild(input);
    }
    input.value = token;
  }

  function applyCsrfToForms(root) {
    var scope = root && root.querySelectorAll ? root : document;
    var forms = scope.querySelectorAll('form');
    forms.forEach(ensureCsrfField);
  }

  function updateOnlineStatus() {
    var banner = document.getElementById('offline-banner');
    if (!banner) {
      return;
    }
    banner.classList.toggle('hidden', navigator.onLine);
  }

  function ensureToastContainer() {
    var container = document.getElementById('toast-container');
    if (container) {
      return container;
    }
    container = document.createElement('div');
    container.id = 'toast-container';
    container.className = 'fixed bottom-20 right-4 z-50 flex max-w-sm flex-col gap-2 sm:bottom-4';
    container.setAttribute('aria-live', 'polite');
    document.body.appendChild(container);
    return container;
  }

  function showToast(message, variant) {
    if (!message) {
      return;
    }
    var tones = {
      error: 'border-destructive/30 bg-destructive/10 text-destructive',
      success: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-700',
      warning: 'border-amber-500/30 bg-amber-500/10 text-amber-700',
      info: 'border-border bg-card text-foreground'
    };
    var toast = document.createElement('div');
    toast.className = 'rounded-sm border px-3 py-2 text-xs font-bold shadow-xl backdrop-blur ' + (tones[variant] || tones.info);
    toast.textContent = message;
    ensureToastContainer().appendChild(toast);
    window.setTimeout(function () {
      toast.remove();
    }, 4000);
  }

  function updateWarningBadges(nextCount) {
    // B310: the sidebar renders this badge twice (mobile drawer + desktop);
    // query by class so both stay in sync.
    var badges = document.querySelectorAll('.warning-badge');
    badges.forEach(function (badge) {
      badge.textContent = String(nextCount);
      badge.classList.toggle('hidden', nextCount <= 0);
    });
  }

  function currentBadgeCount() {
    var firstBadge = document.querySelector('.warning-badge');
    return firstBadge ? parseInt(firstBadge.textContent || '0', 10) || 0 : 0;
  }

  function incrementWarningBadges() {
    updateWarningBadges(currentBadgeCount() + 1);
  }

  function decrementWarningBadges() {
    var currentCount = currentBadgeCount();
    if (currentCount > 0) {
      updateWarningBadges(currentCount - 1);
    }
  }

  function refreshWarningBadgeFromServer() {
    // B310: /warnings/unread-count is the session-authenticated web route;
    // the previous /api/warnings/unread-count path does not exist and made
    // this refresh a perpetual 404.
    // After session expiry the fetch follows the /login redirect and the HTML
    // body parses to 0, silently hiding the badge; `redirect: 'manual'` and
    // the `ok` check make the refresh a no-op instead.
    fetch('/warnings/unread-count', { redirect: 'manual', credentials: 'same-origin' })
      .then(function (res) {
        if (!res.ok) {
          return null;
        }
        return res.text();
      })
      .then(function (count) {
        if (count === null) {
          return;
        }
        updateWarningBadges(parseInt(count, 10) || 0);
      })
      .catch(function () { /* ignore server errors */ });
  }

  document.addEventListener('warning-acknowledged', function () {
    decrementWarningBadges();
    refreshWarningBadgeFromServer();
  });

  // The SSE bridge emits `apex:sse-resync` when a reconnect could not replay
  // missed events; canonical state must replace the local incremental view.
  document.addEventListener('apex:sse-resync', function () {
    refreshWarningBadgeFromServer();
  });

  // Global a11y rule: a scroll container must be keyboard-reachable, or its
  // hidden content is unreachable without a pointer (axe
  // `scrollable-region-focusable`). Applied to every scrollable region,
  // including those injected by HTMX swaps.
  function initScrollableRegions(root) {
    var scope = root || document;
    var selector = '.overflow-x-auto, .overflow-y-auto, [data-scrollable]';
    scope.querySelectorAll(selector).forEach(function (node) {
      if (!node.hasAttribute('tabindex')) {
        node.setAttribute('tabindex', '0');
      }
    });
    if (scope.matches && scope.matches(selector) && !scope.hasAttribute('tabindex')) {
      scope.setAttribute('tabindex', '0');
    }
  }

  document.addEventListener('DOMContentLoaded', function () {
    applyCsrfToForms(document);
    updateOnlineStatus();
    initDenseTableKeyboardNav();
    initDestructiveConfirms();
    initScrollableRegions(document);
    document.body.addEventListener('htmx:afterSwap', function (event) {
      initScrollableRegions(event.target);
    });
    // Pause the refresh while the tab is hidden; a background tab has no
    // visible badge to keep current.
    window.setInterval(function () {
      if (!document.hidden) {
        refreshWarningBadgeFromServer();
      }
    }, 60000);
  });

  // B346: arrow-key row navigation for `[data-dense-table]` tables — the
  // templates advertise "Arrow keys move between rows. Press Enter to open".
  // Rows are queried inside the keydown handler: after an HTMX filter or
  // pagination swap the previously captured nodes are detached.
  function initDenseTableKeyboardNav() {
    var tables = document.querySelectorAll('table[data-dense-table]');
    tables.forEach(function (table) {
      if (table.getAttribute('data-kbd-init') === '1') {
        return;
      }
      table.setAttribute('data-kbd-init', '1');
      var index = -1;

      table.setAttribute('tabindex', '0');
      table.addEventListener('keydown', function (event) {
        var rows = Array.prototype.slice.call(table.querySelectorAll('tbody tr'));
        if (!rows.length) {
          return;
        }
        index = Math.min(index, rows.length - 1);

        function apply() {
          rows.forEach(function (row, i) {
            row.classList.toggle('apex-row-selected', i === index);
          });
          if (index >= 0 && rows[index].scrollIntoView) {
            rows[index].scrollIntoView({ block: 'nearest' });
          }
        }

        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
          event.preventDefault();
          index = event.key === 'ArrowDown'
            ? Math.min(index + 1, rows.length - 1)
            : Math.max(index - 1, 0);
          apply();
        } else if (event.key === 'Enter' && index >= 0) {
          var link = rows[index].querySelector('a[href]');
          if (link) {
            event.preventDefault();
            link.click();
          }
        }
      });
    });
  }

  // B347: confirmation for destructive form submissions. Templates opt in
  // with data-confirm="…"; used by close-workspace, queue complete, triage
  // resolve/acknowledge.
  function initDestructiveConfirms() {
    document.addEventListener('submit', function (event) {
      var form = event.target;
      if (!(form instanceof HTMLFormElement)) { return; }
      var message = form.getAttribute('data-confirm');
      if (message && !window.confirm(message)) {
        event.preventDefault();
        event.stopPropagation();
      }
    }, true);
    // Also cover plain confirmation links.
    document.addEventListener('click', function (event) {
      var link = event.target.closest ? event.target.closest('a[data-confirm]') : null;
      if (link && !window.confirm(link.getAttribute('data-confirm'))) {
        event.preventDefault();
        event.stopPropagation();
      }
    });
  }

  document.addEventListener('submit', function (event) {
    ensureCsrfField(event.target);
  }, true);

  window.addEventListener('online', updateOnlineStatus);
  window.addEventListener('offline', updateOnlineStatus);

  document.addEventListener('htmx:configRequest', function (event) {
    var verb = String(event.detail.verb || 'GET').toUpperCase();
    if (verb === 'GET' || verb === 'HEAD' || verb === 'OPTIONS') {
      return;
    }
    var token = getCsrfToken();
    if (!token) {
      return;
    }
    event.detail.headers['X-CSRF-Token'] = token;
  });

  document.addEventListener('htmx:beforeRequest', function () {
    setRouteProgress(true);
  });

  document.addEventListener('htmx:afterSwap', function (event) {
    applyCsrfToForms(event.target || document);
    initDenseTableKeyboardNav();
    setRouteProgress(false);
  });

  document.addEventListener('htmx:responseError', function (event) {
    setRouteProgress(false);
    showToast('Request failed: ' + (event.detail.xhr.status || 'network error'), 'error');
  });

  document.addEventListener('htmx:sendError', function () {
    setRouteProgress(false);
    showToast('Request failed: network error', 'error');
  });

  // The error toast is owned by `htmx:responseError`/`htmx:sendError` above;
  // showing it here too produced two identical toasts per failure.
  document.addEventListener('htmx:afterRequest', function () {
    setRouteProgress(false);
  });

  // Back/forward from the bfcache restores the page with the bar still active.
  window.addEventListener('pageshow', function () {
    setRouteProgress(false);
  });

  window.showToast = showToast;
  window.apexCsrfToken = getCsrfToken;
})();