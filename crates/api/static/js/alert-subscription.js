/* ── Entity "Watch alerts" control ─────────────────────────────────────────
   Drives GET/PUT/DELETE /api/entities/:id/alert-subscription for the
   `user_alert_subscriptions` table the realtime alert router resolves against.

   "Watch alerts" is deliberately distinct from the other entity actions:
   - Add to work queue  → POST /api/queue        (a personal task)
   - Investigate        → POST /api/workspaces   (opens a workspace)
   - Add to pipeline    → POST /api/executive/opportunities (tracks a deal)
   - Watch alerts       → PUT  /api/entities/:id/alert-subscription (this file)
   ────────────────────────────────────────────────────────────────────────── */
(function () {
  "use strict";

  var SEVERITIES = ["info", "low", "medium", "high", "critical"];

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

  function endpoint(entityId, category) {
    var url = "/api/entities/" + encodeURIComponent(entityId) + "/alert-subscription";
    if (typeof category === "string" && category.length > 0) {
      url += "?category=" + encodeURIComponent(category);
    }
    return url;
  }

  function request(method, url, body) {
    var options = {
      method: method,
      headers: {
        Accept: "application/json",
        "x-csrf-token": getCookie("apex_csrf")
      },
      credentials: "same-origin"
    };
    if (body !== undefined) {
      options.headers["Content-Type"] = "application/json";
      options.body = JSON.stringify(body);
    }
    return fetch(url, options).then(function (response) {
      return response.json().catch(function () {
        return null;
      }).then(function (payload) {
        if (!response.ok) {
          var message = payload && payload.error && payload.error.message
            ? payload.error.message
            : "Request failed (" + response.status + ")";
          var error = new Error(message);
          error.status = response.status;
          throw error;
        }
        return payload && Object.prototype.hasOwnProperty.call(payload, "data") ? payload.data : payload;
      });
    });
  }

  function severityRank(value) {
    var index = SEVERITIES.indexOf(String(value || "").toLowerCase());
    return index === -1 ? 2 : index;
  }

  /* Prefer an enabled all-category subscription, then any enabled row. */
  function pickPrimary(subscriptions) {
    var rows = Array.isArray(subscriptions) ? subscriptions : [];
    var enabled = rows.filter(function (row) { return row && row.enabled; });
    var allCategory = enabled.filter(function (row) { return !row.category; });
    if (allCategory.length) return allCategory[0];
    if (enabled.length) return enabled[0];
    return rows[0] || null;
  }

  function feedback(root, message, tone) {
    var node = root.querySelector("[data-alert-subscription-feedback]");
    if (!node) return;
    node.textContent = message || "";
    node.classList.toggle("apex-text-positive", tone === "success");
    node.classList.toggle("text-rams-red", tone === "error");
  }

  function setWatching(root, watching) {
    var badge = root.querySelector("[data-alert-subscription-state]");
    if (badge) {
      badge.textContent = watching ? "Watching" : "Not watching";
      badge.classList.toggle("apex-text-positive", watching);
    }
    var save = root.querySelector("[data-alert-subscription-save]");
    if (save) {
      save.textContent = watching ? "Update watch" : "Watch alerts";
    }
  }

  function entityLabel(root) {
    return root.getAttribute("data-entity-label") || "this entity";
  }

  function save(root) {
    var entityId = root.getAttribute("data-entity-id");
    if (!entityId) return;
    var severityNode = root.querySelector("[data-alert-subscription-severity]");
    var categoryNode = root.querySelector("[data-alert-subscription-category]");
    var category = categoryNode ? categoryNode.value.trim() : "";
    var payload = {
      enabled: true,
      category: category ? category : null,
      min_severity: severityNode ? severityNode.value : "medium"
    };

    feedback(root, "Saving…", "info");
    request("PUT", endpoint(entityId), payload).then(function (data) {
      var subscription = data && data.subscription ? data.subscription : null;
      root.__apexSubscriptions = subscription ? [subscription] : [];
      setWatching(root, true);
      var message = "Watching " + payload.min_severity + " alerts for " + entityLabel(root)
        + ". This is separate from the work queue, investigations, and the pipeline.";
      feedback(root, message, "success");
      if (typeof window.showToast === "function") {
        window.showToast("Watching alerts for " + entityLabel(root), "success");
      }
    }).catch(function (error) {
      feedback(root, error && error.message ? error.message : "Could not save the alert subscription.", "error");
    });
  }

  function stop(root) {
    var entityId = root.getAttribute("data-entity-id");
    if (!entityId) return;
    var known = Array.isArray(root.__apexSubscriptions) ? root.__apexSubscriptions : [];
    var categories = known.map(function (row) { return row && row.category ? row.category : null; });
    if (!categories.length) categories = [null];

    var deletions = categories.map(function (category) {
      return request("DELETE", endpoint(entityId, category)).catch(function (error) {
        if (error && error.status === 404) return null;
        throw error;
      });
    });

    feedback(root, "Stopping…", "info");
    Promise.all(deletions).then(function () {
      root.__apexSubscriptions = [];
      setWatching(root, false);
      feedback(root, "Stopped watching alerts for " + entityLabel(root) + ".", "success");
      if (typeof window.showToast === "function") {
        window.showToast("Stopped watching alerts for " + entityLabel(root), "info");
      }
    }).catch(function (error) {
      feedback(root, error && error.message ? error.message : "Could not remove the alert subscription.", "error");
    });
  }

  function load(root) {
    var entityId = root.getAttribute("data-entity-id");
    if (!entityId) return;
    request("GET", endpoint(entityId)).then(function (data) {
      var subscriptions = data && Array.isArray(data.subscriptions) ? data.subscriptions : [];
      root.__apexSubscriptions = subscriptions;
      var primary = pickPrimary(subscriptions);
      if (primary) {
        var severityNode = root.querySelector("[data-alert-subscription-severity]");
        var categoryNode = root.querySelector("[data-alert-subscription-category]");
        if (severityNode && primary.min_severity) severityNode.value = String(primary.min_severity).toLowerCase();
        if (categoryNode) categoryNode.value = primary.category || "";
        setWatching(root, Boolean(primary.enabled));
        if (primary.enabled) {
          feedback(root, "Watching this entity at " + primary.min_severity + " severity or higher.", "success");
        }
      } else {
        setWatching(root, false);
      }
    }).catch(function (error) {
      feedback(root, error && error.message ? error.message : "Could not load the alert subscription.", "error");
    });
  }

  function init(root) {
    if (!root || root.__apexAlertSubscription) return;
    root.__apexAlertSubscription = true;
    var saveButton = root.querySelector("[data-alert-subscription-save]");
    var stopButton = root.querySelector("[data-alert-subscription-stop]");
    if (saveButton) saveButton.addEventListener("click", function () { save(root); });
    if (stopButton) stopButton.addEventListener("click", function () { stop(root); });
    load(root);
  }

  function initAll() {
    document.querySelectorAll("[data-alert-subscription]").forEach(init);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", initAll);
  } else {
    initAll();
  }

  window.ApexAlertSubscription = {
    endpoint: endpoint,
    severityRank: severityRank,
    pickPrimary: pickPrimary,
    init: init,
    initAll: initAll
  };
})();
