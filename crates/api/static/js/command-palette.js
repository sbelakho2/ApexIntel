(function () {
  "use strict";

  var RESULT_LIMIT = 12;
  var RECENT_KEY = "apex-command-recent";
  var RECENT_LIMIT = 6;
  var SEARCH_DEBOUNCE_MS = 140;

  var COMMANDS = [
    { id: "nav-overview", label: "Overview", aliases: ["home", "dashboard"], url: "/" },
    { id: "nav-executive", label: "Executive", aliases: ["briefing", "summary"], url: "/executive" },
    { id: "nav-trends", label: "Trends", aliases: ["history", "charts"], url: "/trends" },
    { id: "nav-warnings", label: "Warnings", aliases: ["alerts", "risk"], url: "/warnings" },
    { id: "nav-insights", label: "Insights", aliases: ["intel", "findings"], url: "/insights" },
    { id: "nav-triage", label: "Triage", aliases: ["review", "queue"], url: "/triage" },
    { id: "nav-workspaces", label: "Workspaces", aliases: ["investigations"], url: "/workspaces" },
    { id: "nav-new-workspace", label: "New investigation", aliases: ["investigate", "create workspace"], url: "/workspaces/new" },
    { id: "nav-queue", label: "Priority queue", aliases: ["queue", "tasks"], url: "/queue" },
    { id: "nav-activity", label: "Activity", aliases: ["feed", "events"], url: "/activity" },
    { id: "nav-memos", label: "Memos", aliases: ["notes", "reports"], url: "/memos" },
    { id: "nav-inbox", label: "Inbox", aliases: ["notifications"], url: "/notifications" },
    { id: "nav-companies", label: "Companies", aliases: ["accounts", "entities"], url: "/companies" },
    { id: "nav-persons", label: "Persons", aliases: ["people", "contacts", "poi"], url: "/persons" },
    { id: "nav-competitors", label: "Competitors", aliases: ["rivals"], url: "/competitors" },
    { id: "nav-battlecards", label: "Battlecards", aliases: ["cards", "playbooks"], url: "/battlecards" },
    { id: "nav-security", label: "Security", aliases: ["dns", "kev", "lookalike"], url: "/security" },
    { id: "nav-supplier-risk", label: "Supplier risk", aliases: ["risk", "supply"], url: "/supplier-risk" },
    { id: "nav-pipeline", label: "Pipeline", aliases: ["opportunities", "deals"], url: "/pipeline" },
    { id: "nav-graph", label: "Graph explorer", aliases: ["network", "relationships"], url: "/graph" },
    { id: "nav-recipes", label: "Recipes", aliases: ["rules", "detectors"], url: "/recipes" },
    { id: "nav-search", label: "Search", aliases: ["find", "query"], url: "/search" },
    { id: "nav-settings", label: "Settings", aliases: ["preferences", "config"], url: "/settings" },
    { id: "nav-alert-settings", label: "Alert settings", aliases: ["notifications", "thresholds"], url: "/settings/alerts" },
    { id: "action-toggle-theme", label: "Toggle theme", aliases: ["dark", "light", "night", "day"], run: "toggle-theme" },
  ];

  var ENTITY_LABELS = {
    company: "Company",
    person: "Person",
    warning: "Warning",
    insight: "Insight",
    workspace: "Workspace",
    battlecard: "Battlecard",
    command: "Command",
  };

  var state = {
    open: false,
    results: [],
    activeIndex: -1,
    loading: false,
    token: 0,
    lastFocus: null,
    entityIndex: null,
    entityIndexPromise: null,
  };

  var dom = {};

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

  function entityUrl(type, id) {
    switch (type) {
      case "company":
        return "/companies/" + encodeURIComponent(id);
      case "person":
        return "/persons/" + encodeURIComponent(id);
      case "warning":
        return "/warnings/" + encodeURIComponent(id);
      case "insight":
        return "/insights/" + encodeURIComponent(id);
      case "workspace":
        return "/workspaces/" + encodeURIComponent(id);
      case "battlecard":
        return "/battlecards/" + encodeURIComponent(id);
      default:
        return null;
    }
  }

  function fuzzyScore(query, text) {
    if (!query) return 0;
    var lowerText = String(text || "").toLowerCase();
    if (!lowerText) return -1;
    if (lowerText === query) return 1000;
    if (lowerText.indexOf(query) === 0) return 900;
    var substringAt = lowerText.indexOf(query);
    if (substringAt >= 0) return 800 - Math.min(200, substringAt);
    var textIndex = 0;
    var score = 0;
    var streak = 0;
    for (var queryIndex = 0; queryIndex < query.length; queryIndex += 1) {
      var found = lowerText.indexOf(query[queryIndex], textIndex);
      if (found === -1) return -1;
      if (found === textIndex) {
        streak += 1;
        score += 12 + streak * 4;
      } else {
        streak = 0;
        score += 4;
      }
      textIndex = found + 1;
    }
    return 300 + score;
  }

  function scoreItem(item, query) {
    if (!query) return 0;
    var best = fuzzyScore(query, item.label);
    (item.aliases || []).forEach(function (alias) {
      var aliasScore = fuzzyScore(query, alias) - 60;
      if (aliasScore > best) best = aliasScore;
    });
    return best;
  }

  function parseApiData(payload) {
    if (!payload || typeof payload !== "object") return payload;
    if (Object.prototype.hasOwnProperty.call(payload, "data")) return payload.data;
    return payload;
  }

  function fetchJson(url) {
    return fetch(url, {
      headers: { Accept: "application/json" },
      credentials: "same-origin",
    }).then(function (response) {
      return response.json().catch(function () {
        return null;
      }).then(function (payload) {
        if (!response.ok) {
          var message = payload && payload.error && payload.error.message
            ? payload.error.message
            : "Request failed (" + response.status + ")";
          throw new Error(message);
        }
        return parseApiData(payload);
      });
    });
  }

  function postJson(url, body) {
    return fetch(url, {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
        "x-csrf-token": getCookie("apex_csrf"),
      },
      credentials: "same-origin",
      body: JSON.stringify(body),
    }).then(function (response) {
      return response.json().catch(function () {
        return null;
      }).then(function (payload) {
        if (!response.ok) {
          var message = payload && payload.error && payload.error.message
            ? payload.error.message
            : "Request failed (" + response.status + ")";
          throw new Error(message);
        }
        return parseApiData(payload);
      });
    });
  }

  function readRecent() {
    try {
      var raw = window.localStorage ? window.localStorage.getItem(RECENT_KEY) : null;
      var parsed = raw ? JSON.parse(raw) : [];
      return Array.isArray(parsed) ? parsed : [];
    } catch (error) {
      return [];
    }
  }

  function recordRecent(item) {
    if (!item || !item.url || !item.type) return;
    var key = item.type + ":" + item.id;
    var next = readRecent().filter(function (entry) {
      return entry.type + ":" + entry.id !== key;
    });
    next.unshift({ type: item.type, id: item.id, label: item.label, url: item.url, meta: item.meta || "" });
    next = next.slice(0, RECENT_LIMIT);
    try {
      if (window.localStorage) window.localStorage.setItem(RECENT_KEY, JSON.stringify(next));
    } catch (error) {
      /* storage unavailable */
    }
  }

  function recentItems() {
    return readRecent().map(function (entry) {
      return {
        id: entry.id,
        type: entry.type,
        label: entry.label || entry.id,
        meta: entry.meta || "Recent",
        url: entry.url || entityUrl(entry.type, entry.id),
        group: "Recent",
      };
    }).filter(function (item) {
      return Boolean(item.url);
    });
  }

  function commandItems(query) {
    var scored = COMMANDS.map(function (command) {
      return {
        id: command.id,
        type: "command",
        label: command.label,
        meta: command.url || "Command",
        url: command.url || null,
        aliases: command.aliases || [],
        run: command.run || null,
        group: "Commands",
        score: query ? scoreItem(command, query) : 100,
      };
    }).filter(function (item) {
      return !query || item.score >= 0;
    });
    scored.sort(function (left, right) {
      return right.score - left.score || left.label.localeCompare(right.label);
    });
    return scored;
  }

  function loadEntityIndex() {
    if (state.entityIndexPromise) return state.entityIndexPromise;
    state.entityIndexPromise = Promise.all([
      fetchJson("/api/workspaces?limit=100").catch(function () { return []; }),
      fetchJson("/api/battlecards?per_page=100").catch(function () { return []; }),
    ]).then(function (responses) {
      var workspaces = Array.isArray(responses[0]) ? responses[0] : [];
      var battlecards = Array.isArray(responses[1]) ? responses[1] : [];
      var items = [];
      workspaces.forEach(function (workspace) {
        if (!workspace || !workspace.id) return;
        items.push({
          id: String(workspace.id),
          type: "workspace",
          label: workspace.name || "Workspace",
          meta: [workspace.workspace_type, workspace.status].filter(Boolean).join(" • ") || "Workspace",
          url: entityUrl("workspace", workspace.id),
          group: "Workspaces",
        });
      });
      battlecards.forEach(function (card) {
        if (!card || !card.id) return;
        items.push({
          id: String(card.id),
          type: "battlecard",
          label: card.account_name || card.title || "Battlecard",
          meta: [card.competitor_name, card.status].filter(Boolean).join(" • ") || "Battlecard",
          url: entityUrl("battlecard", card.id),
          group: "Battlecards",
        });
      });
      state.entityIndex = items;
      return items;
    });
    return state.entityIndexPromise;
  }

  function mergeResults(groups, limit) {
    var max = limit || RESULT_LIMIT;
    var seen = {};
    var merged = [];
    groups.forEach(function (items) {
      (items || []).forEach(function (item) {
        if (!item || !item.id) return;
        var key = item.type + ":" + item.id;
        if (seen[key]) return;
        seen[key] = true;
        merged.push(item);
      });
    });
    return merged.slice(0, max);
  }

  function refresh(query) {
    state.token += 1;
    var token = state.token;

    if (!query) {
      state.results = mergeResults([recentItems(), commandItems("")], 40);
      render();
      return;
    }

    var local = commandItems(query);
    if (state.entityIndex) {
      var matched = state.entityIndex
        .map(function (item) {
          return Object.assign({}, item, { score: scoreItem(item, query) });
        })
        .filter(function (item) {
          return item.score >= 0;
        })
        .sort(function (left, right) {
          return right.score - left.score;
        });
      local = local.concat(matched);
    }
    state.results = mergeResults([local]);
    render();

    state.loading = true;
    updateStatus();
    var encoder = typeof query === "string" ? encodeURIComponent(query) : "";

    Promise.all([
      fetchJson("/api/search/suggest?q=" + encoder + "&limit=8").catch(function () { return null; }),
      fetchJson("/api/search?q=" + encoder + "&per_page=12").catch(function () { return null; }),
      loadEntityIndex(),
    ]).then(function (responses) {
      if (token !== state.token) return;
      var suggestions = responses[0] && Array.isArray(responses[0].suggestions) ? responses[0].suggestions : [];
      var searchResults = responses[1] && Array.isArray(responses[1].results) ? responses[1].results : [];
      var indexed = state.entityIndex || [];
      var suggestItems = suggestions.map(function (suggestion) {
        var type = String(suggestion.entity_type || "").toLowerCase();
        return {
          id: String(suggestion.id || suggestion.text),
          type: type,
          label: suggestion.text || suggestion.id,
          meta: suggestion.subtext || ENTITY_LABELS[type] || "Result",
          url: entityUrl(type, suggestion.id),
          group: "Suggestions",
          score: Number(suggestion.score || 0) + 2000,
        };
      }).filter(function (item) {
        return Boolean(item.url);
      });
      var searchItems = searchResults.map(function (result) {
        var type = String(result.entity_type || "").toLowerCase();
        return {
          id: String(result.id),
          type: type,
          label: result.title || result.id,
          meta: result.snippet || ENTITY_LABELS[type] || "Result",
          url: result.url || entityUrl(type, result.id),
          group: ENTITY_LABELS[type] ? ENTITY_LABELS[type] + "s" : "Results",
          score: 1000,
        };
      }).filter(function (item) {
        return Boolean(item.url);
      });
      var fuzzyIndexed = indexed
        .map(function (item) {
          return Object.assign({}, item, { score: scoreItem(item, query) });
        })
        .filter(function (item) {
          return item.score >= 0;
        })
        .sort(function (left, right) {
          return right.score - left.score;
        });
      state.loading = false;
      state.results = mergeResults([suggestItems, searchItems, local, fuzzyIndexed]);
      render();
    }).catch(function () {
      state.loading = false;
      render();
    });
  }

  function updateStatus() {
    if (!dom.status) return;
    if (state.loading) {
      dom.status.textContent = "Searching…";
      return;
    }
    var count = state.results.length;
    dom.status.textContent = count === 0 ? "No results" : count + (count === 1 ? " result" : " results");
  }

  function activeItem() {
    return state.results[state.activeIndex] || null;
  }

  function actionsFor(item) {
    if (!item || item.type === "command") return [];
    var actions = [{ key: "open", label: "Open" }];
    if (["company", "person", "warning", "insight"].includes(item.type)) {
      actions.push({ key: "watch", label: "Watch" });
      actions.push({ key: "investigate", label: "Investigate" });
      actions.push({ key: "pipeline", label: "Add to pipeline" });
    }
    return actions;
  }

  function recordAndOpen(item) {
    recordRecent(item);
    if (item.url) {
      window.location.assign(item.url);
      return;
    }
    if (item.run === "toggle-theme") {
      window.ApexTheme && window.ApexTheme.cycleTheme && window.ApexTheme.cycleTheme();
      close();
    }
  }

  function actionRequest(action, item) {
    if (action === "watch") {
      return postJson("/api/queue", {
        item_type: item.type,
        item_id: item.id,
        item_title: item.label,
        priority: 5,
        notes: "Added from command palette",
      }).then(function () {
        return "Added " + item.label + " to the priority queue.";
      });
    }
    if (action === "investigate") {
      return postJson("/api/workspaces", {
        name: "Investigation — " + item.label,
        workspace_type: "structured",
        visibility: "team",
        tags: ["command-palette"],
        entity_focus: [{ type: item.type, id: item.id, label: item.label }],
      }).then(function () {
        return "Opened an investigation workspace for " + item.label + ".";
      });
    }
    if (action === "pipeline") {
      return postJson("/api/executive/opportunities", {
        title: "Opportunity — " + item.label,
        opportunity_type: "expansion",
        entity_id: item.id,
        entity_type: item.type,
        confidence: 0.5,
        priority_score: 0.5,
      }).then(function () {
        return "Added " + item.label + " to the pipeline.";
      });
    }
    return Promise.resolve("");
  }

  function runAction(action, item) {
    if (!item) return;
    if (action === "open") {
      recordAndOpen(item);
      return;
    }
    if (dom.status) dom.status.textContent = "Working…";
    actionRequest(action, item).then(function (message) {
      recordRecent(item);
      if (dom.status) dom.status.textContent = message;
      if (typeof window.showToast === "function") {
        window.showToast(message, "success");
      }
    }).catch(function (error) {
      var message = error && error.message ? error.message : "Action failed";
      if (dom.status) dom.status.textContent = message;
      if (typeof window.showToast === "function") {
        window.showToast(message, "error");
      }
    });
  }

  function setActive(index, options) {
    if (!state.results.length) {
      state.activeIndex = -1;
      renderActive();
      return;
    }
    var next = index;
    if (next < 0) next = state.results.length - 1;
    if (next >= state.results.length) next = 0;
    state.activeIndex = next;
    renderActive();
    if (!options || options.scroll !== false) {
      var option = document.getElementById(optionId(state.activeIndex));
      if (option && typeof option.scrollIntoView === "function") {
        option.scrollIntoView({ block: "nearest" });
      }
    }
  }

  function moveActive(delta) {
    if (!state.results.length) return;
    setActive(state.activeIndex < 0 ? (delta > 0 ? 0 : state.results.length - 1) : state.activeIndex + delta);
  }

  function optionId(index) {
    return "command-option-" + index;
  }

  function renderActive() {
    if (!dom.list) return;
    var options = Array.from(dom.list.querySelectorAll('[role="option"]'));
    options.forEach(function (option, index) {
      var isActive = index === state.activeIndex;
      option.setAttribute("aria-selected", String(isActive));
      option.classList.toggle("command-palette-option-active", isActive);
      Array.from(option.querySelectorAll("button")).forEach(function (button) {
        button.tabIndex = isActive ? 0 : -1;
      });
    });
    if (dom.input) {
      dom.input.setAttribute("aria-activedescendant", state.activeIndex >= 0 ? optionId(state.activeIndex) : "");
      dom.input.setAttribute("aria-expanded", String(state.results.length > 0));
    }
    updateStatus();
  }

  function render() {
    if (!dom.list || !dom.input) return;
    dom.list.innerHTML = "";
    if (!state.results.length) {
      var empty = document.createElement("li");
      empty.className = "apex-empty-state";
      empty.setAttribute("role", "presentation");
      var title = document.createElement("p");
      title.className = "apex-empty-state-title";
      title.textContent = state.loading ? "Searching…" : "No matches";
      var body = document.createElement("p");
      body.className = "apex-empty-state-body";
      body.textContent = state.loading
        ? "Querying companies, persons, warnings, insights, workspaces, and battlecards."
        : "Try a different term, an alias (people, alerts, deals), or a command name.";
      empty.appendChild(title);
      empty.appendChild(body);
      dom.list.appendChild(empty);
      renderActive();
      return;
    }

    var lastGroup = null;
    state.results.forEach(function (item, index) {
      if (item.group && item.group !== lastGroup) {
        lastGroup = item.group;
        var header = document.createElement("li");
        header.className = "command-palette-group";
        header.setAttribute("role", "presentation");
        header.textContent = item.group;
        dom.list.appendChild(header);
      }

      var option = document.createElement("li");
      option.id = optionId(index);
      option.className = "command-palette-option";
      option.setAttribute("role", "option");
      option.setAttribute("aria-selected", "false");
      option.setAttribute("data-index", String(index));

      var main = document.createElement("button");
      main.type = "button";
      main.className = "command-palette-option-main";
      main.setAttribute("data-action", "open");
      main.setAttribute("aria-label", "Open " + item.label);
      main.tabIndex = -1;

      var label = document.createElement("span");
      label.className = "command-palette-option-label";
      label.textContent = item.label;
      var meta = document.createElement("span");
      meta.className = "command-palette-option-meta";
      meta.textContent = item.meta || ENTITY_LABELS[item.type] || "";
      main.appendChild(label);
      main.appendChild(meta);
      main.addEventListener("click", function () {
        runAction("open", item);
      });
      main.addEventListener("mouseenter", function () {
        setActive(index, { scroll: false });
      });
      option.appendChild(main);

      var actions = actionsFor(item);
      if (actions.length > 1) {
        var actionRow = document.createElement("span");
        actionRow.className = "command-palette-option-actions";
        actions.forEach(function (action) {
          var button = document.createElement("button");
          button.type = "button";
          button.className = "apex-chip apex-chip-idle";
          button.textContent = action.label;
          button.setAttribute("data-action", action.key);
          button.setAttribute("aria-label", action.label + " " + item.label);
          button.tabIndex = -1;
          button.addEventListener("click", function (event) {
            event.stopPropagation();
            runAction(action.key, item);
          });
          button.addEventListener("mouseenter", function () {
            setActive(index, { scroll: false });
          });
          actionRow.appendChild(button);
        });
        option.appendChild(actionRow);
      }

      dom.list.appendChild(option);
    });

    if (state.activeIndex < 0 || state.activeIndex >= state.results.length) {
      state.activeIndex = 0;
    }
    renderActive();
  }

  function open(trigger) {
    if (!dom.backdrop) return;
    state.open = true;
    state.lastFocus = trigger || document.activeElement;
    dom.backdrop.hidden = false;
    dom.backdrop.classList.remove("hidden");
    document.body.classList.add("overflow-hidden");
    setTriggersExpanded(true);
    refresh("");
    loadEntityIndex();
    if (dom.input) {
      dom.input.value = "";
      dom.input.focus();
    }
  }

  function close() {
    if (!dom.backdrop) return;
    state.open = false;
    dom.backdrop.classList.add("hidden");
    dom.backdrop.hidden = true;
    document.body.classList.remove("overflow-hidden");
    setTriggersExpanded(false);
    if (state.lastFocus && typeof state.lastFocus.focus === "function") {
      state.lastFocus.focus();
    }
  }

  function setTriggersExpanded(expanded) {
    document.querySelectorAll("[data-command-palette-trigger]").forEach(function (trigger) {
      trigger.setAttribute("aria-expanded", String(expanded));
    });
  }

  function onInputKeydown(event) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveActive(1);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      moveActive(-1);
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      setActive(0);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      setActive(state.results.length - 1);
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      var item = activeItem();
      if (!item) return;
      if (event.altKey) {
        runAction(event.shiftKey ? "pipeline" : "investigate", item);
      } else {
        runAction("open", item);
      }
    }
  }

  function trapTab(event) {
    if (event.key !== "Tab" || !dom.dialog) return;
    var focusables = Array.from(dom.dialog.querySelectorAll('button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]')).filter(function (element) {
      if (element.tabIndex < 0) return false;
      return element.offsetParent !== null || element === document.activeElement;
    });
    if (!focusables.length) return;
    var first = focusables[0];
    var last = focusables[focusables.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function init() {
    dom.backdrop = document.getElementById("command-palette-backdrop");
    dom.dialog = document.getElementById("command-palette");
    dom.input = document.getElementById("command-palette-input");
    dom.list = document.getElementById("command-palette-results");
    dom.status = document.getElementById("command-palette-status");
    dom.close = document.getElementById("command-palette-close");
    if (!dom.backdrop || !dom.dialog || !dom.input || !dom.list) return;

    document.querySelectorAll("[data-command-palette-trigger]").forEach(function (trigger) {
      trigger.addEventListener("click", function () {
        open(trigger);
      });
    });

    dom.close.addEventListener("click", close);

    dom.backdrop.addEventListener("mousedown", function (event) {
      if (event.target === dom.backdrop) close();
    });

    var debounceHandle = 0;
    dom.input.addEventListener("input", function () {
      var value = dom.input.value.trim();
      if (debounceHandle) window.clearTimeout(debounceHandle);
      debounceHandle = window.setTimeout(function () {
        debounceHandle = 0;
        state.activeIndex = -1;
        refresh(value);
      }, SEARCH_DEBOUNCE_MS);
    });

    dom.input.addEventListener("keydown", onInputKeydown);
    dom.dialog.addEventListener("keydown", trapTab);

    document.addEventListener("keydown", function (event) {
      var key = String(event.key || "").toLowerCase();
      if ((event.metaKey || event.ctrlKey) && key === "k") {
        event.preventDefault();
        if (state.open) {
          close();
        } else {
          open(document.activeElement);
        }
        return;
      }
      if (event.key === "Escape" && state.open) {
        event.preventDefault();
        close();
      }
      if (state.open && event.altKey && !event.metaKey && !event.ctrlKey) {
        if (key === "w") {
          event.preventDefault();
          runAction("watch", activeItem());
        } else if (key === "i") {
          event.preventDefault();
          runAction("investigate", activeItem());
        } else if (key === "p") {
          event.preventDefault();
          runAction("pipeline", activeItem());
        }
      }
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
