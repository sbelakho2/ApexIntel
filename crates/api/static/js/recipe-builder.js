// recipe-builder.js — Dynamic form management for create-recipe page.
// Handles add/remove of signals, transforms, thresholds, actions sections,
// AI-assisted generation, and recipe testing.
(function () {
  "use strict";

  const SOURCE_TYPES = [
    "observation","crawl","rss","social","trade_show","patent",
    "tender","certification","sanction","academic",
  ];
  const ENTITY_TYPES = ["company","person","product","location","any"];
  const MATCH_MODES = ["any","all","regex"];
  const TRANSFORM_KINDS = [
    "count","sum","average","max","min","rate_of_change",
    "z_score","moving_average","changepoint",
  ];
  const AGGREGATION_MODES = ["sum","avg","max","min","count","latest"];
  const OPERATORS = [">","<",">=","<=","==","!="];
  const ACTION_KINDS = ["warning","webhook","email","slack","memo","dossier_update"];

  const inputCls =
    "rounded-sm border border-border bg-background px-2 py-1 text-xs focus:outline-none focus:ring-1 focus:ring-primary w-full";
  const selectCls =
    "rounded-sm border border-border bg-background px-2 py-1 text-xs focus:outline-none focus:ring-1 focus:ring-primary";

  function init() {
    const form = document.getElementById("recipe-form");
    if (!form) return;

    wireSection("signals", renderSignal);
    wireSection("transforms", renderTransform);
    wireSection("thresholds", renderThreshold);
    wireSection("actions", renderAction);

    // AI generate button
    const aiBtn = document.getElementById("ai-generate-btn");
    const aiInput = document.getElementById("ai-prompt");
    if (aiBtn && aiInput) {
      aiBtn.addEventListener("click", () => aiGenerate(aiInput.value));
    }

    // Test button
    const testBtn = document.getElementById("test-recipe-btn");
    if (testBtn) {
      testBtn.addEventListener("click", () => testRecipe());
    }

    // Form submission → JSON POST
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      submitRecipe();
    });

    // Initialise with one of each
    addRow("signals", renderSignal);
    addRow("transforms", renderTransform);
    addRow("thresholds", renderThreshold);
    addRow("actions", renderAction);
  }

  // ── Section management ─────────────────────────────────────────────────

  function wireSection(name, renderer) {
    const addBtn = document.querySelector(`[data-add="${name}"]`);
    if (addBtn) addBtn.addEventListener("click", () => addRow(name, renderer));
  }

  function addRow(name, renderer) {
    const container = document.getElementById(`${name}-list`);
    if (!container) return;
    const row = renderer();
    container.appendChild(row);
    updateCount(name);
  }

  function removeRow(el, name) {
    el.closest("[data-row]").remove();
    updateCount(name);
  }

  function updateCount(name) {
    const counter = document.querySelector(`[data-count="${name}"]`);
    const container = document.getElementById(`${name}-list`);
    if (counter && container) {
      counter.textContent = container.querySelectorAll("[data-row]").length;
    }
  }

  // ── Row renderers ──────────────────────────────────────────────────────

  function renderSignal() {
    const row = el("div", "flex items-start gap-2 rounded-md border border-border/50 bg-muted/30 p-2");
    row.dataset.row = "signal";
    const grid = el("div", "grid grid-cols-2 sm:grid-cols-4 gap-2 flex-1");
    grid.append(
      labelSelect("Source", "source_type", SOURCE_TYPES, "observation"),
      labelSelect("Entity", "entity_type", ENTITY_TYPES, "company"),
      labelInput("Keywords", "keywords", "comma-separated"),
      labelSelect("Match", "match_mode", MATCH_MODES, "any")
    );
    row.append(grid, removeBtn("signals"));
    return row;
  }

  function renderTransform() {
    const row = el("div", "flex items-start gap-2 rounded-md border border-border/50 bg-muted/30 p-2");
    row.dataset.row = "transform";
    const grid = el("div", "grid grid-cols-3 gap-2 flex-1");
    grid.append(
      labelSelect("Kind", "kind", TRANSFORM_KINDS, "count"),
      labelNumber("Window (days)", "window_days", 7, 1, 365),
      labelSelect("Aggregation", "aggregation", AGGREGATION_MODES, "sum")
    );
    row.append(grid, removeBtn("transforms"));
    return row;
  }

  function renderThreshold() {
    const row = el("div", "flex items-start gap-2 rounded-md border border-border/50 bg-muted/30 p-2");
    row.dataset.row = "threshold";
    const grid = el("div", "grid grid-cols-3 gap-2 flex-1");
    grid.append(
      labelInput("Metric", "metric", "e.g. count"),
      labelSelect("Operator", "operator", OPERATORS, ">"),
      labelNumber("Value", "value", 5, null, null, 0.1)
    );
    row.append(grid, removeBtn("thresholds"));
    return row;
  }

  function renderAction() {
    const row = el("div", "flex items-start gap-2 rounded-md border border-border/50 bg-muted/30 p-2");
    row.dataset.row = "action";
    const grid = el("div", "grid grid-cols-2 gap-2 flex-1");
    grid.append(
      labelSelect("Action", "action_kind", ACTION_KINDS, "warning"),
      labelInput("Target", "target", "")
    );
    row.append(grid, removeBtn("actions"));
    return row;
  }

  // ── Field helpers ──────────────────────────────────────────────────────

  function labelSelect(text, name, options, def) {
    const wrap = el("label", "flex flex-col gap-0.5");
    wrap.append(spanLabel(text));
    const sel = document.createElement("select");
    sel.name = name;
    sel.className = selectCls;
    options.forEach((o) => {
      const opt = document.createElement("option");
      opt.value = o;
      opt.textContent = o;
      if (o === def) opt.selected = true;
      sel.appendChild(opt);
    });
    wrap.appendChild(sel);
    return wrap;
  }

  function labelInput(text, name, placeholder) {
    const wrap = el("label", "flex flex-col gap-0.5");
    wrap.append(spanLabel(text));
    const inp = document.createElement("input");
    inp.type = "text";
    inp.name = name;
    inp.placeholder = placeholder || "";
    inp.className = inputCls;
    wrap.appendChild(inp);
    return wrap;
  }

  function labelNumber(text, name, def, min, max, step) {
    const wrap = el("label", "flex flex-col gap-0.5");
    wrap.append(spanLabel(text));
    const inp = document.createElement("input");
    inp.type = "number";
    inp.name = name;
    inp.value = def;
    if (min != null) inp.min = min;
    if (max != null) inp.max = max;
    if (step != null) inp.step = step;
    inp.className = inputCls + " w-24";
    wrap.appendChild(inp);
    return wrap;
  }

  function spanLabel(text) {
    const s = document.createElement("span");
    s.className = "text-[9px] font-bold uppercase text-muted-foreground";
    s.textContent = text;
    return s;
  }

  function removeBtn(sectionName) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className =
      "rounded-sm bg-red-500/10 px-1.5 py-0.5 text-[9px] font-bold text-red-400 hover:bg-red-500/20 transition-colors mt-3";
    btn.textContent = "✕";
    btn.addEventListener("click", function () {
      removeRow(this, sectionName);
    });
    return btn;
  }

  function el(tag, cls) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    return e;
  }

  // ── Collect form data → JSON ───────────────────────────────────────────

  function collectForm() {
    const f = document.getElementById("recipe-form");
    const val = (name) => (f.querySelector(`[name="${name}"]`)?.value || "").trim();
    return {
      name: val("name"),
      description: val("description"),
      severity: val("severity"),
      cooldown_hours: Number(val("cooldown_hours")) || 24,
      enabled: f.querySelector('[name="enabled"]')?.checked ?? true,
      narrative_template: val("narrative_template"),
      signals: collectSection("signals-list", (row) => ({
        source_type: row.querySelector('[name="source_type"]')?.value,
        entity_type: row.querySelector('[name="entity_type"]')?.value,
        keywords: row.querySelector('[name="keywords"]')?.value,
        match_mode: row.querySelector('[name="match_mode"]')?.value,
      })),
      transforms: collectSection("transforms-list", (row) => ({
        kind: row.querySelector('[name="kind"]')?.value,
        window_days: Number(row.querySelector('[name="window_days"]')?.value) || 7,
        aggregation: row.querySelector('[name="aggregation"]')?.value,
      })),
      thresholds: collectSection("thresholds-list", (row) => ({
        metric: row.querySelector('[name="metric"]')?.value,
        operator: row.querySelector('[name="operator"]')?.value,
        value: Number(row.querySelector('[name="value"]')?.value) || 0,
      })),
      actions: collectSection("actions-list", (row) => ({
        kind: row.querySelector('[name="action_kind"]')?.value,
        target: row.querySelector('[name="target"]')?.value,
      })),
    };
  }

  function collectSection(containerId, mapper) {
    const c = document.getElementById(containerId);
    if (!c) return [];
    return Array.from(c.querySelectorAll("[data-row]")).map(mapper);
  }

  // ── API calls ──────────────────────────────────────────────────────────

  function csrfJsonHeaders() {
    const headers = { "Content-Type": "application/json" };
    const token = window.apexCsrfToken?.();
    if (token) {
      headers["X-CSRF-Token"] = token;
    }
    return headers;
  }

  function submitRecipe() {
    const data = collectForm();
    if (!data.name || !data.signals.length || !data.thresholds.length || !data.actions.length) {
      window.showToast?.("Name, at least 1 signal, 1 threshold, and 1 action required.", "error");
      return;
    }
    const btn = document.querySelector('[type="submit"]');
    if (btn) { btn.disabled = true; btn.textContent = "Creating…"; }

    fetch("/recipes/create", {
      method: "POST",
      headers: csrfJsonHeaders(),
      body: JSON.stringify(data),
    })
      .then((r) => {
        if (!r.ok) throw new Error("Create failed");
        return r.json();
      })
      .then(() => {
        window.showToast?.(`Recipe "${data.name}" created.`, "success");
        window.location.href = "/recipes";
      })
      .catch((e) => {
        window.showToast?.(e.message, "error");
        if (btn) { btn.disabled = false; btn.textContent = "Create Recipe"; }
      });
  }

  function testRecipe() {
    const data = collectForm();
    const btn = document.getElementById("test-recipe-btn");
    if (btn) { btn.disabled = true; btn.textContent = "Testing…"; }

    fetch("/recipes/test", {
      method: "POST",
      headers: csrfJsonHeaders(),
      body: JSON.stringify(data),
    })
      .then((r) => {
        if (!r.ok) throw new Error("Test failed");
        return r.json();
      })
      .then((res) => {
        const panel = document.getElementById("test-results");
        if (panel) {
          panel.classList.remove("hidden");
          panel.innerHTML = `
            <h4 class="text-xs font-bold text-green-400 mb-1">
              Test Results: ${res.matches} matching observation${res.matches !== 1 ? "s" : ""}
            </h4>
            ${(res.sample_warnings || []).map((s) => `<p class="text-[10px] text-green-300/70 truncate">${escHtml(s)}</p>`).join("")}
            ${res.matches === 0 ? '<p class="text-[10px] text-muted-foreground">No matches. Broaden keywords or lower thresholds.</p>' : ""}`;
        }
      })
      .catch((e) => window.showToast?.(e.message, "error"))
      .finally(() => {
        if (btn) { btn.disabled = false; btn.textContent = "Test Recipe"; }
      });
  }

  function aiGenerate(prompt) {
    if (!prompt?.trim()) return;
    const btn = document.getElementById("ai-generate-btn");
    if (btn) { btn.disabled = true; btn.textContent = "Generating…"; }

    const payload = buildAiRecipePayload(prompt);

    fetch("/llm/generate-recipe", {
      method: "POST",
      headers: csrfJsonHeaders(),
      body: JSON.stringify(payload),
    })
      .then((r) => {
        if (!r.ok) throw new Error("AI generation failed");
        return r.json();
      })
      .then((res) => {
        if (!res?.success || !res?.data) {
          const msg = res?.error?.message || "AI generation failed";
          throw new Error(msg);
        }
        const recipe = res.data.recipe_json || {};

        // Populate basic fields
        setField("description", prompt.trim());
        if (recipe.id) setField("name", humanizeRecipeId(recipe.id));
        if (recipe.narrative_template) setField("narrative_template", recipe.narrative_template);

        // Replace dynamic sections
        const generatedSignals = mapGeneratedSignals(recipe.signals);
        if (generatedSignals.length) replaceSectionRows("signals", generatedSignals, renderSignal, (row, data) => {
          setInRow(row, "source_type", data.source_type);
          setInRow(row, "entity_type", data.entity_type);
          setInRow(row, "keywords", data.keywords);
          setInRow(row, "match_mode", data.match_mode);
        });

        const generatedActions = mapGeneratedActions(recipe.action_playbook);
        if (generatedActions.length) replaceSectionRows("actions", generatedActions, renderAction, (row, data) => {
          setInRow(row, "action_kind", data.kind);
          setInRow(row, "target", data.target);
        });

        window.showToast?.("Recipe generated by AI. Review and adjust.", "info");
      })
      .catch((e) => window.showToast?.(e.message, "error"))
      .finally(() => {
        if (btn) { btn.disabled = false; btn.textContent = "Generate with AI"; }
      });
  }

  function buildAiRecipePayload(prompt) {
    const description = (prompt || "").trim();
    const inferredSignals = inferSignals(description);
    return {
      pattern_description: description,
      outcome: inferOutcome(description),
      signals: inferredSignals.length ? inferredSignals : ["emerging_signal"],
      existing_recipe_ids: [],
      regions: [],
    };
  }

  function inferOutcome(prompt) {
    const slug = toSnake(prompt).replace(/^_+|_+$/g, "").slice(0, 48);
    return slug || "emerging_risk_pattern";
  }

  function inferSignals(prompt) {
    if (!prompt) return [];
    const segments = prompt
      .split(/[;,\n]+/)
      .map((s) => s.trim())
      .filter(Boolean);

    const normalized = segments
      .map((s) => toSnake(s).replace(/^_+|_+$/g, ""))
      .filter((s) => s.length >= 3);

    if (normalized.length) {
      return Array.from(new Set(normalized)).slice(0, 8);
    }

    return prompt
      .toLowerCase()
      .replace(/[^a-z0-9\s]/g, " ")
      .split(/\s+/)
      .filter((w) => w.length >= 4)
      .slice(0, 6)
      .map((w) => toSnake(w));
  }

  function mapGeneratedSignals(signals) {
    if (!Array.isArray(signals)) return [];
    return signals
      .map((s) => {
        const name = (s?.name || "").trim();
        const description = (s?.description || "").trim();
        const keywords = [name, description].filter(Boolean).join(", ");
        if (!keywords) return null;
        return {
          source_type: "observation",
          entity_type: inferEntityType(keywords),
          keywords,
          match_mode: "any",
        };
      })
      .filter(Boolean);
  }

  function mapGeneratedActions(actionPlaybook) {
    if (!actionPlaybook || typeof actionPlaybook !== "string") return [];
    return [{ kind: "warning", target: actionPlaybook.trim() }];
  }

  function inferEntityType(text) {
    const t = (text || "").toLowerCase();
    if (t.includes("person") || t.includes("executive") || t.includes("officer")) return "person";
    if (t.includes("company") || t.includes("supplier") || t.includes("vendor") || t.includes("firm")) return "company";
    if (t.includes("product") || t.includes("component") || t.includes("device")) return "product";
    if (t.includes("region") || t.includes("country") || t.includes("city") || t.includes("port")) return "location";
    return "any";
  }

  function humanizeRecipeId(id) {
    const source = (id || "").toString().trim();
    if (!source) return "";
    return source
      .replace(/[_-]+/g, " ")
      .replace(/\b\w/g, (c) => c.toUpperCase())
      .slice(0, 120);
  }

  function toSnake(value) {
    return (value || "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "_")
      .replace(/^_+|_+$/g, "");
  }

  function setField(name, value) {
    const el = document.querySelector(`#recipe-form [name="${name}"]`);
    if (el) el.value = value;
  }

  function setInRow(row, name, value) {
    const el = row.querySelector(`[name="${name}"]`);
    if (el && value != null) el.value = value;
  }

  function replaceSectionRows(name, dataArray, renderer, filler) {
    const container = document.getElementById(`${name}-list`);
    if (!container) return;
    container.innerHTML = "";
    dataArray.forEach((data) => {
      const row = renderer();
      filler(row, data);
      container.appendChild(row);
    });
    updateCount(name);
  }

  function escHtml(s) {
    const e = document.createElement("span");
    e.textContent = s;
    return e.innerHTML;
  }

  // Boot
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
