/**
 * ApexIntel SSE Client — Real-time alert streaming via EventSource.
 *
 * Connects to the SSE endpoint at /api/v1/events/stream and dispatches
 * events to registered handlers. Supports automatic reconnection with
 * exponential backoff and Last-Event-ID tracking.
 *
 * Usage:
 *   const client = new ApexIntelSSE();
 *   client.on('new_warning', (data) => { ... });
 *   client.on('new_insight', (data) => { ... });
 */
class ApexIntelSSE {
  /**
   * @param {Object} options
   * @param {string} [options.url='/api/v1/events/stream'] - SSE endpoint URL
   * @param {number} [options.reconnectDelay=3000] - Initial reconnection delay (ms)
   * @param {number} [options.maxReconnectDelay=30000] - Maximum reconnection delay (ms)
   * @param {Function} [options.onOpen] - Called when connection opens
   * @param {Function} [options.onError] - Called on connection error
   */
  constructor(options = {}) {
    this.url = options.url || '/api/v1/events/stream';
    this.reconnectDelay = options.reconnectDelay || 3000;
    this.maxReconnectDelay = options.maxReconnectDelay || 30000;
    this.onOpenCallback = options.onOpen || null;
    this.onErrorCallback = options.onError || null;

    /** @type {Object.<string, Function[]>} */
    this.eventHandlers = {};
    /** @type {EventSource|null} */
    this.eventSource = null;
    /** @type {number} */
    this.currentReconnectDelay = this.reconnectDelay;
    /** @type {boolean} */
    this.intentionalClose = false;
    /** @type {number|null} */
    this.reconnectTimer = null;

    this.connect();
  }

  /**
   * Establish the SSE connection.
   * @private
   */
  connect() {
    if (this.eventSource) {
      this.eventSource.close();
    }

    this.eventSource = new EventSource(this.url);
    this.intentionalClose = false;

    this.eventSource.onopen = () => {
      this.currentReconnectDelay = this.reconnectDelay;
      if (typeof this.onOpenCallback === 'function') {
        this.onOpenCallback();
      }
    };

    this.eventSource.onerror = () => {
      if (this.intentionalClose) {
        return;
      }

      if (typeof this.onErrorCallback === 'function') {
        this.onErrorCallback();
      }

      this.scheduleReconnect();
    };

    // Generic message handler dispatches by event type
    this.eventSource.onmessage = (event) => {
      this.dispatch('message', event.data);
    };

    // Re-attach every registered named handler to this EventSource. `on()`
    // attaches handlers to the *current* source; without this, all named
    // subscriptions are silently lost after a reconnect.
    this.attachAllHandlers();
  }

  /**
   * Schedule a reconnection attempt with exponential backoff.
   * @private
   */
  scheduleReconnect() {
    if (this.intentionalClose) {
      return;
    }

    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
    }

    this.reconnectTimer = setTimeout(() => {
      if (!this.intentionalClose) {
        this.connect();
      }
    }, this.currentReconnectDelay);

    // Exponential backoff with jitter
    this.currentReconnectDelay = Math.min(
      this.currentReconnectDelay * 1.5 + Math.random() * 1000,
      this.maxReconnectDelay
    );
  }

  /**
   * Register an event handler for a specific event type.
   *
   * @param {string} eventType - The SSE event type (e.g., 'new_warning', 'new_insight')
   * @param {Function} handler - Callback receiving parsed JSON data
   * @returns {ApexIntelSSE} - For chaining
   */
  on(eventType, handler) {
    if (!this.eventHandlers[eventType]) {
      this.eventHandlers[eventType] = [];
    }
    this.eventHandlers[eventType].push(handler);
    this.attachHandler(eventType, handler);

    return this;
  }

  /**
   * Attach a single handler to the current EventSource.
   * @private
   */
  attachHandler(eventType, handler) {
    if (!this.eventSource) {
      return;
    }
    this.eventSource.addEventListener(eventType, (event) => {
      try {
        const data = JSON.parse(event.data);
        handler(data);
      } catch (e) {
        console.warn(`ApexIntelSSE: Failed to parse event data for '${eventType}':`, e);
      }
    });
  }

  /**
   * Re-attach every stored handler to the current EventSource.
   * @private
   */
  attachAllHandlers() {
    Object.keys(this.eventHandlers).forEach((eventType) => {
      this.eventHandlers[eventType].forEach((handler) => {
        this.attachHandler(eventType, handler);
      });
    });
  }

  /**
   * Remove an event handler.
   *
   * @param {string} eventType
   * @param {Function} handler
   */
  off(eventType, handler) {
    if (!this.eventHandlers[eventType]) {
      return;
    }
    const idx = this.eventHandlers[eventType].indexOf(handler);
    if (idx !== -1) {
      this.eventHandlers[eventType].splice(idx, 1);
    }
  }

  /**
   * Dispatch an event to all registered handlers.
   * @private
   */
  dispatch(eventType, data) {
    const handlers = this.eventHandlers[eventType];
    if (!handlers) {
      return;
    }
    for (const handler of handlers) {
      try {
        handler(data);
      } catch (e) {
        console.warn(`ApexIntelSSE: Handler error for '${eventType}':`, e);
      }
    }
  }

  /**
   * Close the SSE connection intentionally (no reconnect).
   */
  disconnect() {
    this.intentionalClose = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
    }
  }

  // ── Convenience handlers ─────────────────────────────────────────────

  /**
   * Handle an incoming warning alert.
   * Override this method or use .on('new_warning', handler).
   */
  handleWarning(data) {
    this.showNotification(data);
    this.updateBadge(data);
  }

  /**
   * Handle an incoming insight alert.
   * Override this method or use .on('new_insight', handler).
   */
  handleInsight(data) {
    this.showNotification(data);
    this.updateBadge(data);
  }

  /**
   * Show a browser/desktop notification if permission is granted.
   */
  showNotification(alert) {
    if (!('Notification' in window)) {
      return;
    }

    if (Notification.permission === 'granted') {
      this.createNotification(alert);
    } else if (Notification.permission !== 'denied') {
      Notification.requestPermission().then((permission) => {
        if (permission === 'granted') {
          this.createNotification(alert);
        }
      });
    }
  }

  /**
   * @private
   */
  createNotification(alert) {
    const title = alert.title || 'ApexIntel Alert';
    const options = {
      body: alert.description || '',
      icon: '/static/icons/icon-192.svg',
      tag: `apex-alert-${alert.id || Date.now()}`,
    };

    try {
      const notification = new Notification(title, options);
      notification.onclick = () => {
        window.focus();
        if (alert.entity_id) {
          window.location.href = `/warnings/${alert.entity_id}`;
        } else {
          window.location.href = '/warnings';
        }
        notification.close();
      };

      // Auto-close after 8 seconds
      setTimeout(() => notification.close(), 8000);
    } catch (e) {
      // Silently fail if Notification fails
    }
  }

  /**
   * Update the notification badge count in the navigation.
   */
  updateBadge(alert) {
    // B310: the badge is rendered twice (mobile drawer + desktop sidebar) —
    // update every instance by class instead of only the first by id.
    const badges = document.querySelectorAll('.warning-badge');
    if (!badges.length) {
      return;
    }

    const first = badges[0];
    const currentCount = parseInt(first.textContent || '0', 10) || 0;
    const newCount = currentCount + 1;
    badges.forEach((badge) => {
      badge.textContent = String(newCount);
      badge.classList.remove('hidden');
      badge.style.display = 'inline';
    });
  }

  /**
   * Show an in-page toast notification.
   */
  showToast(alert) {
    const toast = document.getElementById('notification-toast');
    if (!toast) {
      return;
    }

    const typeEl = toast.querySelector('.apex-toast-type');
    const titleEl = toast.querySelector('.apex-toast-title');
    const descEl = toast.querySelector('.apex-toast-description');

    if (typeEl) {
      typeEl.textContent = (alert.severity || 'info').toUpperCase();
    }
    if (titleEl) {
      titleEl.textContent = alert.title || '';
    }
    if (descEl) {
      descEl.textContent = alert.description || '';
    }

    // Apply severity color
    const severity = (alert.severity || '').toLowerCase();
    toast.classList.remove(
      'apex-toast-critical',
      'apex-toast-warning',
      'apex-toast-info'
    );
    if (severity === 'critical') {
      toast.classList.add('apex-toast-critical');
    } else if (severity === 'high' || severity === 'medium') {
      toast.classList.add('apex-toast-warning');
    } else {
      toast.classList.add('apex-toast-info');
    }

    toast.style.display = 'block';

    // Auto-dismiss after 8 seconds
    if (this._toastTimer) {
      clearTimeout(this._toastTimer);
    }
    this._toastTimer = setTimeout(() => {
      toast.style.display = 'none';
    }, 8000);
  }
}

// ── Auto-initialize on page load if body has data-sse attribute ────────
(function () {
  const body = document.body;
  if (!body || !body.hasAttribute('data-sse')) {
    return;
  }

  const sse = new ApexIntelSSE({
    url: body.getAttribute('data-sse-url') || '/api/v1/events/stream',
  });

  // Store on window for external access
  window.__apexSSE = sse;

  // Register default event handlers
  sse.on('new_warning', (data) => {
    sse.handleWarning(data);
    sse.showToast(data);
  });

  sse.on('new_insight', (data) => {
    sse.handleInsight(data);
    sse.showToast(data);
  });

  sse.on('recipe_match', (data) => {
    sse.showToast(data);
  });

  sse.on('competitor_change', (data) => {
    sse.showToast(data);
  });

  sse.on('system_alert', (data) => {
    sse.showToast(data);
  });
})();
