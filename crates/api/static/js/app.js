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
        return decodeURIComponent(cookie.slice(prefix.length));
      }
    }
    return '';
  }

  function getCsrfToken() {
    return getCookie('apex_csrf');
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
    var badges = document.querySelectorAll('[data-warning-badge]');
    badges.forEach(function (badge) {
      badge.textContent = String(nextCount);
      badge.classList.toggle('hidden', nextCount <= 0);
    });
  }

  function incrementWarningBadges() {
    var firstBadge = document.querySelector('[data-warning-badge]');
    var currentCount = firstBadge ? parseInt(firstBadge.textContent || '0', 10) || 0 : 0;
    updateWarningBadges(currentCount + 1);
  }

  function decrementWarningBadges() {
    var firstBadge = document.querySelector('[data-warning-badge]');
    var currentCount = firstBadge ? parseInt(firstBadge.textContent || '0', 10) || 0 : 0;
    if (currentCount > 0) {
      updateWarningBadges(currentCount - 1);
    }
  }

  function refreshWarningBadgeFromServer() {
    fetch('/api/warnings/unread-count')
      .then(function (res) { return res.text(); })
      .then(function (count) {
        updateWarningBadges(parseInt(count, 10) || 0);
      })
      .catch(function () { /* ignore server errors */ });
  }

  function connectWarningsWs() {
    if (!document.querySelector('[data-ws-warnings]')) {
      return;
    }
    var protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    var retryDelay = 1000;

    function openSocket() {
      var socket = new WebSocket(protocol + '//' + window.location.host + '/ws/warnings');
      socket.onopen = function () {
        retryDelay = 1000;
      };
      socket.onmessage = function (event) {
        try {
          var payload = JSON.parse(event.data);
          incrementWarningBadges();
          showToast(payload.title || 'New warning detected', 'warning');
        } catch (error) {
          incrementWarningBadges();
        }
      };
      socket.onerror = function () {
        socket.close();
      };
      socket.onclose = function () {
        window.setTimeout(function () {
          retryDelay = Math.min(retryDelay * 2, 30000);
          openSocket();
        }, retryDelay);
      };
    }

    openSocket();
  }

  document.addEventListener('warning-acknowledged', function () {
    decrementWarningBadges();
    refreshWarningBadgeFromServer();
  });

  document.addEventListener('DOMContentLoaded', function () {
    applyCsrfToForms(document);
    updateOnlineStatus();
    connectWarningsWs();
    window.setInterval(refreshWarningBadgeFromServer, 60000);
  });

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

  document.addEventListener('htmx:afterRequest', function (event) {
    if (event.detail.failed) {
      showToast('Request failed: ' + (event.detail.xhr.status || 'network error'), 'error');
    }
  });

  window.showToast = showToast;
  window.apexCsrfToken = getCsrfToken;
})();