// graph.js — force-directed graph runtime used by /graph

(function () {
  "use strict";

  const NODE_COLORS = {
    company: "#4A90E2",
    person: "#2D8C3C",
    country: "#FFBE00",
    region: "#FFBE00",
    cert: "#4A90E2",
    tender: "#D62D2D",
    domain: "#D62D2D",
    warning: "#D62D2D",
    insight: "#FFBE00",
  };

  const NODE_RADIUS = {
    company: 14,
    person: 11,
    country: 10,
    region: 10,
    cert: 9,
    tender: 9,
    domain: 9,
    warning: 9,
    insight: 9,
  };

  const MIN_SCALE = 0.15;
  const MAX_SCALE = 8;

  function normalizeType(value) {
    const normalized = String(value || "domain").toLowerCase();
    if (normalized.includes("company") || normalized === "org" || normalized === "organization") return "company";
    if (normalized.includes("person") || normalized.includes("poi")) return "person";
    if (normalized.includes("country") || normalized.includes("nation") || normalized.includes("state")) return "country";
    if (normalized.includes("region") || normalized.includes("geo") || normalized.includes("location") || normalized.includes("site")) return "region";
    if (normalized.includes("cert") || normalized.includes("certificate") || normalized.includes("compliance")) return "cert";
    if (normalized.includes("tender") || normalized.includes("procurement") || normalized.includes("contract") || normalized.includes("rfp")) return "tender";
    if (normalized.includes("domain") || normalized.includes("dns")) return "domain";
    return normalized;
  }

  function normalizePayload(payload) {
    const nodeMap = {};
    const nodes = (payload.nodes || []).map((node, index) => {
      const type = normalizeType(node.type || node.node_type);
      const normalized = {
        id: String(node.id),
        label: String(node.label || node.id || "unknown"),
        type,
        size: Number(node.size || 10),
        color: node.color || NODE_COLORS[type] || "#888",
        x: Number.isFinite(Number(node.x)) ? Number(node.x) : undefined,
        y: Number.isFinite(Number(node.y)) ? Number(node.y) : undefined,
        clusterKey: String(node.clusterKey || type),
        activityDays: Number(node.activityDays || 3650),
        _index: index,
      };
      nodeMap[normalized.id] = normalized;
      return normalized;
    });

    const edges = (payload.edges || []).map((edge) => ({
      source: String(edge.source),
      target: String(edge.target),
      weight: Number(edge.weight || 1),
      type: String(edge.type || edge.edge_type || "related_to"),
    }));

    return { nodes, edges, nodeMap };
  }

  function updateSelectedPanel(node) {
    const panel = document.getElementById("graph-selected-node");
    const labelEl = document.getElementById("graph-selected-label");
    const typeEl = document.getElementById("graph-selected-type");
    const linkEl = document.getElementById("graph-selected-link");
    if (!panel || !labelEl || !typeEl) return;
    if (!node) {
      panel.classList.add("hidden");
      return;
    }
    labelEl.textContent = node.label;
    typeEl.textContent = node.type.charAt(0).toUpperCase() + node.type.slice(1);
    // Add navigation link for navigable entity types
    if (linkEl) {
      const href = getNodeHref(node);
      if (href) {
        linkEl.href = href;
        linkEl.textContent = normalizeType(node.type) === "company" ? "View Company →" : "View Person →";
        linkEl.classList.remove("hidden");
      } else {
        linkEl.href = "#";
        linkEl.classList.add("hidden");
      }
    }
    panel.classList.remove("hidden");
  }

  function getNodeHref(node) {
    if (!node || !node.id) return null;
    const ntype = normalizeType(node.type);
    const rawId = String(node.id);
    const uuidMatch = rawId.match(/[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/i);
    const entityId = uuidMatch ? uuidMatch[0] : rawId;

    if (ntype === "company" || rawId.toLowerCase().startsWith("company:")) return "/companies/" + entityId;
    if (ntype === "person" || rawId.toLowerCase().startsWith("person:")) return "/persons/" + entityId;
    return null;
  }

  function updateFilterUI(activeType) {
    const activeFilterEl = document.getElementById("graph-active-filter");
    const filterChips = Array.from(document.querySelectorAll(".graph-filter-chip"));
    if (activeFilterEl) activeFilterEl.textContent = "Active: " + (activeType ? "1" : "0");
    filterChips.forEach((chip) => {
      const isActive = chip.getAttribute("data-type") === activeType;
      chip.classList.toggle("border-primary", isActive);
      chip.classList.toggle("bg-primary/10", isActive);
      chip.classList.toggle("text-foreground", isActive);
      chip.classList.toggle("border-border", !isActive);
      chip.classList.toggle("bg-secondary", !isActive);
      chip.classList.toggle("text-muted-foreground", !isActive);
    });
  }

  function init() {
    const container = document.getElementById("graph-container");
    if (!container) return;
    if (container.__graphInitialized) return;
    container.__graphInitialized = true;

    const embeddedRaw = container.dataset.graphJson;
    if (embeddedRaw) {
      try {
        const payload = JSON.parse(embeddedRaw);
        return initWithData(container, payload);
      } catch (error) {
        console.error("[graph] invalid embedded JSON", error);
      }
    }

    const url = container.dataset.graphUrl || "/api/graph";
    fetch(url, { headers: { Accept: "application/json" }, credentials: "include" })
      .then((r) => r.json())
      .then((json) => initWithData(container, json.data || json))
      .catch((err) => {
        console.error("[graph] fetch failed", err);
        container.innerHTML = '<p class="text-muted-foreground text-sm p-4">Could not load graph data.</p>';
      });
  }

  function initWithData(container, payload) {
    const all = normalizePayload(payload || {});
    let activeType = null;
    let searchText = "";
    let clusterMode = false;
    let activityWindow = "all";

    const filterChips = Array.from(document.querySelectorAll(".graph-filter-chip"));
    const resetBtn = document.getElementById("graph-reset-filters");
    const searchInput = document.getElementById("graph-search-input");
    const activityFilter = document.getElementById("graph-activity-filter");
    const clusterToggle = document.getElementById("graph-cluster-toggle");
    const exportJsonBtn = document.getElementById("graph-export-json");
    const exportSvgBtn = document.getElementById("graph-export-svg");
    const visibleCountEl = document.getElementById("graph-visible-count");

    const rerender = function () {
      let nodes = activeType
        ? all.nodes.filter((node) => node.type === activeType)
        : all.nodes.slice();

      if (searchText) {
        const lowered = searchText.toLowerCase();
        nodes = nodes.filter((node) => node.label.toLowerCase().includes(lowered) || node.id.toLowerCase().includes(lowered));
      }

      if (activityWindow !== "all") {
        const maxDays = Number(activityWindow);
        nodes = nodes.filter((node) => Number(node.activityDays || 3650) <= maxDays);
      }

      const nodeSet = new Set(nodes.map((node) => node.id));
      const edges = all.edges.filter((edge) => nodeSet.has(edge.source) && nodeSet.has(edge.target));
      if (visibleCountEl) visibleCountEl.textContent = "Visible: " + nodes.length;
      updateFilterUI(activeType);
      render(container, nodes, edges, { clusterMode });
    };

    filterChips.forEach((chip) => {
      chip.addEventListener("click", function () {
        const type = chip.getAttribute("data-type");
        activeType = activeType === type ? null : type;
        updateSelectedPanel(null);
        rerender();
      });
    });

    if (resetBtn) {
      resetBtn.addEventListener("click", function () {
        activeType = null;
        searchText = "";
        clusterMode = false;
        activityWindow = "all";
        if (searchInput) searchInput.value = "";
        if (activityFilter) activityFilter.value = "all";
        if (clusterToggle) clusterToggle.textContent = "Cluster by Type";
        updateSelectedPanel(null);
        rerender();
      });
    }

    if (searchInput) {
      searchInput.addEventListener("input", function () {
        searchText = String(searchInput.value || "").trim();
        rerender();
      });
    }

    if (activityFilter) {
      activityFilter.addEventListener("change", function () {
        activityWindow = activityFilter.value || "all";
        rerender();
      });
    }

    if (clusterToggle) {
      clusterToggle.addEventListener("click", function () {
        clusterMode = !clusterMode;
        clusterToggle.textContent = clusterMode ? "Clustered" : "Cluster by Type";
        rerender();
      });
    }

    if (exportJsonBtn) {
      exportJsonBtn.addEventListener("click", function () {
        const data = container.__graphExportData;
        if (!data) return;
        downloadText("graph-export.json", JSON.stringify(data, null, 2), "application/json");
      });
    }

    if (exportSvgBtn) {
      exportSvgBtn.addEventListener("click", function () {
        const svg = container.querySelector("svg");
        if (!svg) return;
        downloadText("graph-export.svg", new XMLSerializer().serializeToString(svg), "image/svg+xml");
      });
    }

    rerender();
  }

  function render(container, rawNodes, rawEdges, options) {
    if (typeof container.__graphCleanup === "function") container.__graphCleanup();
    const clusterMode = Boolean(options && options.clusterMode);

    container.innerHTML = "";
    const w = container.clientWidth || 800;
    const h = container.clientHeight || 550;

    let scale = 1;
    let panX = 0;
    let panY = 0;
    let dragging = false;
    let dragNode = null;
    let dragStart = { x: 0, y: 0, px: 0, py: 0 };
    let hoveredId = null;
    let selectedId = null;
    let disposed = false;

    const nodes = rawNodes.map((node, index) => {
      const centers = {
        company: [w * 0.30, h * 0.48],
        person: [w * 0.70, h * 0.48],
        country: [w * 0.52, h * 0.18],
        region: [w * 0.22, h * 0.18],
        cert: [w * 0.18, h * 0.72],
        tender: [w * 0.78, h * 0.72],
        domain: [w * 0.50, h * 0.82],
      };
      const angle = (index / Math.max(1, rawNodes.length)) * Math.PI * 2;
      const orbit = clusterMode ? Math.min(w, h) * 0.08 : Math.min(w, h) * 0.28;
      const baseCenter = clusterMode ? (centers[node.clusterKey] || [w / 2, h / 2]) : [w / 2, h / 2];
      return {
        ...node,
        x: Number.isFinite(node.x) ? node.x : baseCenter[0] + Math.cos(angle) * orbit,
        y: Number.isFinite(node.y) ? node.y : baseCenter[1] + Math.sin(angle) * orbit,
        vx: 0,
        vy: 0,
      };
    });

    container.__graphExportData = {
      nodes: rawNodes,
      edges: rawEdges,
      options: { clusterMode },
    };

    const nodeById = {};
    nodes.forEach((node) => {
      nodeById[node.id] = node;
    });

    const ns = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(ns, "svg");
    svg.setAttribute("width", String(w));
    svg.setAttribute("height", String(h));
    svg.style.display = "block";
    svg.style.width = "100%";
    svg.style.height = "100%";
    svg.style.cursor = "grab";
    svg.style.touchAction = "none";
    container.appendChild(svg);

    const rootG = document.createElementNS(ns, "g");
    const edgeG = document.createElementNS(ns, "g");
    const nodeG = document.createElementNS(ns, "g");
    rootG.appendChild(edgeG);
    rootG.appendChild(nodeG);
    svg.appendChild(rootG);

    const edgeEls = rawEdges.map((edge) => {
      const line = document.createElementNS(ns, "line");
      line.setAttribute("stroke", "#4B5563");
      line.setAttribute("stroke-width", String(Math.max(1, edge.weight * 0.5)));
      line.setAttribute("opacity", "0.45");
      if (edge.type === "competes_with") line.setAttribute("stroke-dasharray", "4 2");
      edgeG.appendChild(line);
      return { el: line, data: edge };
    });

    const nodeEls = nodes.map((node) => {
      const group = document.createElementNS(ns, "g");
      group.style.cursor = "pointer";
      group.dataset.nodeid = node.id;
      group.setAttribute("tabindex", "0");
      group.setAttribute("role", "button");
      group.setAttribute("aria-label", `${node.type}: ${node.label}`);

      const circle = document.createElementNS(ns, "circle");
      const baseRadius = Math.max(9, Math.round((node.size || 12) * 0.65));
      circle.setAttribute("r", String(baseRadius));
      circle.setAttribute("fill", node.color || NODE_COLORS[node.type] || "#888");
      circle.setAttribute("stroke", "transparent");
      circle.setAttribute("stroke-width", "2");
      circle.style.transition = "r 0.15s, opacity 0.15s";
      group.appendChild(circle);

      const label = document.createElementNS(ns, "text");
      label.setAttribute("text-anchor", "middle");
      label.setAttribute("fill", "currentColor");
      label.setAttribute("font-size", "10");
      label.setAttribute("font-weight", "700");
      label.setAttribute("class", "uppercase");
      label.style.pointerEvents = "none";
      label.style.userSelect = "none";
      label.style.display = "";
      label.textContent = node.label.length > 14 ? node.label.slice(0, 13) + "…" : node.label;
      group.appendChild(label);

      group.addEventListener("mouseenter", () => setHover(node.id));
      group.addEventListener("mouseleave", () => setHover(null));
      group.addEventListener("focus", () => setHover(node.id));
      group.addEventListener("blur", () => setHover(null));
      group.addEventListener("click", () => selectNode(node.id));
      group.addEventListener("dblclick", () => {
        const href = getNodeHref(node);
        if (href) {
          window.location.assign(href);
        }
      });
      group.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          selectNode(node.id);
        }
      });
      group.addEventListener("mousedown", (event) => {
        if (event.button !== 0) return;
        event.stopPropagation();
        dragNode = node.id;
        dragging = true;
        dragStart = { x: event.clientX, y: event.clientY, px: node.x, py: node.y };
      });

      nodeG.appendChild(group);
      return { g: group, circle, label, data: node, baseRadius };
    });

    addControls(container, () => {
      scale = Math.min(MAX_SCALE, scale * 1.3);
      applyTransform();
    }, () => {
      scale = Math.max(MIN_SCALE, scale / 1.3);
      applyTransform();
    }, () => {
      scale = 1;
      panX = 0;
      panY = 0;
      applyTransform();
    });

    const badge = document.createElement("div");
    badge.className = "absolute left-4 top-4 rounded bg-black/40 px-2 py-0.5 text-[10px] text-muted-foreground backdrop-blur";
    badge.textContent = "100%";
    container.appendChild(badge);

    const legend = document.createElement("div");
    legend.className = "absolute bottom-4 left-4 flex flex-wrap gap-3 text-xs";
    Object.entries(NODE_COLORS).forEach(([type, color]) => {
      if (!nodes.some((node) => node.type === type)) return;
      const item = document.createElement("div");
      item.className = "flex items-center gap-1.5";
      item.innerHTML = `<span class="inline-block h-3 w-3 rounded-full" style="background:${color}"></span><span class="capitalize text-muted-foreground">${type}</span>`;
      legend.appendChild(item);
    });
    container.appendChild(legend);

    function setHover(id) {
      hoveredId = id;
      updateVisuals();
      if (hoveredId && nodeById[hoveredId]) {
        updateSelectedPanel(nodeById[hoveredId]);
      } else if (selectedId && nodeById[selectedId]) {
        updateSelectedPanel(nodeById[selectedId]);
      } else {
        updateSelectedPanel(null);
      }
    }

    function selectNode(id) {
      selectedId = selectedId === id ? null : id;
      updateVisuals();
      if (selectedId && nodeById[selectedId]) {
        updateSelectedPanel(nodeById[selectedId]);
      } else if (hoveredId && nodeById[hoveredId]) {
        updateSelectedPanel(nodeById[hoveredId]);
      } else {
        updateSelectedPanel(null);
      }
    }

    function updateVisuals() {
      nodeEls.forEach(({ circle, label, data, baseRadius }) => {
        const isHovered = hoveredId === data.id;
        const isSelected = selectedId === data.id;
        circle.setAttribute("r", String(isHovered || isSelected ? baseRadius * 1.2 : baseRadius));
        circle.setAttribute("stroke", isSelected ? "#000" : isHovered ? "#ffffff" : "transparent");
        circle.setAttribute("opacity", hoveredId && !isHovered && !isSelected ? "0.3" : "1");
        label.setAttribute("y", String(baseRadius + 12));
        label.style.opacity = hoveredId && !isHovered && !isSelected ? "0.35" : "1";
      });

      edgeEls.forEach(({ el, data }) => {
        const lit = hoveredId === data.source || hoveredId === data.target || selectedId === data.source || selectedId === data.target;
        el.setAttribute("stroke", lit ? "#333" : "#ccc");
        el.setAttribute("opacity", hoveredId || selectedId ? (lit ? "0.85" : "0.08") : "0.45");
      });
    }

    function applyTransform() {
      rootG.setAttribute("transform", `translate(${panX},${panY}) scale(${scale})`);
      badge.textContent = Math.round(scale * 100) + "%";
      edgeEls.forEach(({ el, data }) => {
        el.setAttribute("stroke-width", String(Math.max(1, data.weight * 0.5) / scale));
      });
      nodeEls.forEach(({ label }) => {
        label.setAttribute("font-size", String(Math.max(8, Math.round(10 / scale))));
      });
    }

    svg.addEventListener("wheel", (event) => {
      event.preventDefault();
      const rect = svg.getBoundingClientRect();
      const mx = event.clientX - rect.left;
      const my = event.clientY - rect.top;
      const factor = event.deltaY < 0 ? 1.12 : 1 / 1.12;
      const prev = scale;
      scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, prev * factor));
      const ratio = scale / prev;
      panX = mx - (mx - panX) * ratio;
      panY = my - (my - panY) * ratio;
      applyTransform();
    }, { passive: false });

    svg.addEventListener("mousedown", (event) => {
      if (event.button !== 0) return;
      if (event.target.closest("[data-nodeid]")) return;
      dragging = true;
      dragNode = null;
      dragStart = { x: event.clientX, y: event.clientY, px: panX, py: panY };
      svg.style.cursor = "grabbing";
    });

    const onMouseMove = (event) => {
      if (!dragging) return;
      if (dragNode) {
        const node = nodeById[dragNode];
        if (!node) return;
        node.x = dragStart.px + (event.clientX - dragStart.x) / scale;
        node.y = dragStart.py + (event.clientY - dragStart.y) / scale;
        node.vx = 0;
        node.vy = 0;
      } else {
        panX = dragStart.px + (event.clientX - dragStart.x);
        panY = dragStart.py + (event.clientY - dragStart.y);
        applyTransform();
      }
    };

    const onMouseUp = () => {
      dragging = false;
      dragNode = null;
      svg.style.cursor = "grab";
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);

    let frame = 0;
    const centerX = w / 2;
    const centerY = h / 2;

    function simulate() {
      if (disposed) return;

      nodes.forEach((node) => {
        node.vx += (centerX - node.x) * 0.0007;
        node.vy += (centerY - node.y) * 0.0007;

        nodes.forEach((other) => {
          if (node.id === other.id) return;
          const dx = node.x - other.x;
          const dy = node.y - other.y;
          const dist = Math.sqrt(dx * dx + dy * dy) || 1;
          const force = 5000 / (dist * dist);
          node.vx += (dx / dist) * force * 0.01;
          node.vy += (dy / dist) * force * 0.01;
        });
      });

      rawEdges.forEach((edge) => {
        const source = nodeById[edge.source];
        const target = nodeById[edge.target];
        if (!source || !target) return;
        const dx = target.x - source.x;
        const dy = target.y - source.y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 1;
        const force = (dist - 120) * 0.001 * edge.weight;
        source.vx += (dx / dist) * force;
        source.vy += (dy / dist) * force;
        target.vx -= (dx / dist) * force;
        target.vy -= (dy / dist) * force;
      });

      nodes.forEach((node) => {
        if (dragNode === node.id) return;
        node.x += node.vx;
        node.y += node.vy;
        node.vx *= 0.88;
        node.vy *= 0.88;
      });

      nodeEls.forEach(({ g, data }) => {
        g.setAttribute("transform", `translate(${data.x},${data.y})`);
      });

      edgeEls.forEach(({ el, data }) => {
        const source = nodeById[data.source];
        const target = nodeById[data.target];
        if (!source || !target) return;
        el.setAttribute("x1", String(source.x));
        el.setAttribute("y1", String(source.y));
        el.setAttribute("x2", String(target.x));
        el.setAttribute("y2", String(target.y));
      });

      frame += 1;
      if (frame < 500 || nodes.some((node) => Math.abs(node.vx) > 0.04 || Math.abs(node.vy) > 0.04)) {
        requestAnimationFrame(simulate);
      }
    }

    applyTransform();
    updateVisuals();
    requestAnimationFrame(simulate);

    container.__graphCleanup = function () {
      disposed = true;
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      container.__graphInitialized = false;
    };
  }

  function addControls(container, onIn, onOut, onReset) {
    const wrap = document.createElement("div");
    wrap.className = "absolute right-4 top-4 flex flex-col gap-1";
    [["+", "Zoom in", onIn], ["−", "Zoom out", onOut], ["⊙", "Reset view", onReset]].forEach(([text, title, fn]) => {
      const btn = document.createElement("button");
      btn.className = "flex h-7 w-7 items-center justify-center rounded border border-border bg-card text-sm font-bold text-foreground shadow hover:bg-accent";
      btn.textContent = text;
      btn.title = title;
      btn.addEventListener("click", fn);
      wrap.appendChild(btn);
    });
    container.appendChild(wrap);
  }

  function downloadText(fileName, text, mimeType) {
    const blob = new Blob([text], { type: mimeType });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = fileName;
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    setTimeout(function () { URL.revokeObjectURL(url); }, 0);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }

  // Reinitialize graph when HTMX swaps the graph page content.
  document.addEventListener("htmx:afterSwap", function (event) {
    const target = event && event.detail ? event.detail.target : null;
    if (!target) return;
    if (target.id === "main-results" || target.querySelector("#graph-container")) {
      init();
    }
  });
})();
