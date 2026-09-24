// graph.js — force-directed graph runtime used by /graph

(function () {
  "use strict";

  const NODE_COLORS = {
    company: "#2563EB",
    person: "#059669",
    country: "#D97706",
    region: "#D97706",
    cert: "#7C3AED",
    tender: "#DC2626",
    domain: "#0891B2",
    warning: "#DC2626",
    insight: "#7C3AED",
  };

  // B350: theme-aware stage colors. The previous constants hardcoded a white
  // board with dark strokes, rendering an unreadable white panel in dark
  // mode regardless of the Rams tokens.
  const DARK_MODE_QUERY = window.matchMedia
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;

  function isDarkMode() {
    if (document.documentElement.classList.contains("dark")) {
      return true;
    }
    return !!(DARK_MODE_QUERY && DARK_MODE_QUERY.matches);
  }

  const GRAPH_THEME = {
    get stageTop() {
      return isDarkMode() ? "rgba(11,16,24,0.96)" : "rgba(255,255,255,0.96)";
    },
    get stageBottom() {
      return isDarkMode() ? "rgba(17,24,37,0.95)" : "rgba(236,242,248,0.95)";
    },
    neutralStroke: "#5B6878",
    get subduedLabel() {
      return isDarkMode() ? "#8B99AC" : "#526071";
    },
    get selectedStroke() {
      return isDarkMode() ? "#F2F5F9" : "#17212B";
    },
    get hoverStroke() {
      return isDarkMode() ? "#F2F5F9" : "#FFFFFF";
    },
    matchStroke: "#D97706",
    defaultNode: "#8A96A8",
  };

  const EDGE_STYLES = {
    related_to: { color: "#718096", dash: "" },
    supplier_of: { color: "#059669", dash: "" },
    competes_with: { color: "#DC2626", dash: "6 4" },
    subsidiary_of: { color: "#2563EB", dash: "" },
    associated_with: { color: "#2563EB", dash: "2 4" },
  };

  const CLUSTER_LAYOUT = {
    company: { x: 0.26, y: 0.40 },
    person: { x: 0.76, y: 0.36 },
    country: { x: 0.54, y: 0.14 },
    region: { x: 0.18, y: 0.22 },
    cert: { x: 0.18, y: 0.74 },
    tender: { x: 0.82, y: 0.70 },
    domain: { x: 0.58, y: 0.84 },
    warning: { x: 0.84, y: 0.22 },
    insight: { x: 0.48, y: 0.58 },
  };

  const MIN_SCALE = 0.15;
  const MAX_SCALE = 8;

  function stableHash(value) {
    let hash = 0;
    const text = String(value || "");
    for (let index = 0; index < text.length; index += 1) {
      hash = ((hash << 5) - hash) + text.charCodeAt(index);
      hash |= 0;
    }
    return Math.abs(hash);
  }

  function seededOffset(id, radius) {
    const hash = stableHash(id);
    const angle = (hash % 360) * (Math.PI / 180);
    const distance = radius * (0.35 + ((hash >> 3) % 100) / 140);
    return {
      x: Math.cos(angle) * distance,
      y: Math.sin(angle) * distance,
    };
  }

  function buildClusterAnchors(nodes, width, height, clusterMode) {
    const anchors = {};
    const discovered = new Set(nodes.map((node) => node.clusterKey));
    discovered.forEach((clusterKey, index) => {
      const layout = CLUSTER_LAYOUT[clusterKey] || {
        x: 0.5 + Math.cos((index / Math.max(1, discovered.size)) * Math.PI * 2) * 0.24,
        y: 0.5 + Math.sin((index / Math.max(1, discovered.size)) * Math.PI * 2) * 0.24,
      };
      anchors[clusterKey] = {
        x: width * layout.x,
        y: height * layout.y,
        radius: Math.min(width, height) * (clusterMode ? 0.14 : 0.20),
      };
    });
    return anchors;
  }

  function adjacencyFromEdges(nodes, edges) {
    const adjacency = {};
    nodes.forEach((node) => {
      adjacency[node.id] = [];
    });
    edges.forEach((edge) => {
      if (adjacency[edge.source]) adjacency[edge.source].push(edge.target);
      if (adjacency[edge.target]) adjacency[edge.target].push(edge.source);
    });
    return adjacency;
  }

  function buildRelationshipLayout(nodes, adjacency, degreeIndex, width, height) {
    const nodeIds = new Set(nodes.map((node) => node.id));
    const unseen = new Set(nodeIds);
    const components = [];

    while (unseen.size > 0) {
      const startId = unseen.values().next().value;
      const queue = [startId];
      const component = [];
      unseen.delete(startId);

      while (queue.length) {
        const currentId = queue.shift();
        component.push(currentId);
        (adjacency[currentId] || []).forEach((neighborId) => {
          if (!nodeIds.has(neighborId) || !unseen.has(neighborId)) return;
          unseen.delete(neighborId);
          queue.push(neighborId);
        });
      }

      components.push(component);
    }

    components.sort((left, right) => right.length - left.length);

    const positions = {};
    const anchors = {};
    const componentByNode = {};
    const horizontalOrbit = Math.min(width * 0.26, 190);
    const verticalOrbit = Math.min(height * 0.18, 120);

    components.forEach((component, componentIndex) => {
      const angle = components.length === 1 ? -Math.PI / 2 : ((componentIndex / components.length) * Math.PI * 2) - (Math.PI / 2);
      const center = {
        x: components.length === 1 ? width * 0.5 : width * 0.5 + Math.cos(angle) * horizontalOrbit,
        y: components.length === 1 ? height * 0.5 : height * 0.5 + Math.sin(angle) * verticalOrbit,
      };
      anchors[componentIndex] = center;

      const ordered = component.slice().sort((leftId, rightId) => {
        const degreeDiff = (degreeIndex[rightId] || 0) - (degreeIndex[leftId] || 0);
        if (degreeDiff !== 0) return degreeDiff;
        return stableHash(leftId) - stableHash(rightId);
      });

      ordered.forEach((nodeId, localIndex) => {
        componentByNode[nodeId] = componentIndex;
        if (localIndex === 0) {
          const jitter = seededOffset(nodeId, 10);
          positions[nodeId] = {
            x: center.x + jitter.x,
            y: center.y + jitter.y,
          };
          return;
        }

        const ringIndex = Math.floor((localIndex - 1) / 6);
        const slotIndex = (localIndex - 1) % 6;
        const slotCount = Math.min(12, Math.max(6, component.length - 1));
        const degree = degreeIndex[nodeId] || 0;
        const baseRadius = 58 + (ringIndex * 40);
        const importancePull = Math.min(20, degree * 3.2);
        const angleOffset = (stableHash(nodeId) % 360) * (Math.PI / 180) * 0.16;
        const nodeAngle = ((slotIndex / slotCount) * Math.PI * 2) + angleOffset;
        const jitter = seededOffset(nodeId, 12 + (ringIndex * 4));
        positions[nodeId] = {
          x: center.x + Math.cos(nodeAngle) * Math.max(34, baseRadius - importancePull) + jitter.x,
          y: center.y + Math.sin(nodeAngle) * Math.max(34, baseRadius - importancePull) + jitter.y,
        };
      });
    });

    return { positions, anchors, componentByNode };
  }

  function edgeTargetDistance(edge, sameCluster) {
    switch (edge.type) {
      case "subsidiary_of":
        return sameCluster ? 88 : 110;
      case "supplier_of":
        return sameCluster ? 104 : 128;
      case "competes_with":
        return sameCluster ? 132 : 164;
      case "associated_with":
        return sameCluster ? 114 : 140;
      default:
        return sameCluster ? 118 : 148;
    }
  }

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

  // Mirrors the server-side edge vocabulary (web/graph.rs normalize_edge_type)
  // so stored legacy values ("CompanyPerson", "customer_of") still match the
  // progressive-disclosure categories and EDGE_STYLES keys.
  const EDGE_TYPE_ALIASES = {
    companyperson: "associated_with",
    company_person: "associated_with",
    leads: "associated_with",
    led_by: "associated_with",
    manages: "associated_with",
    affiliated_with: "associated_with",
    affiliated: "associated_with",
    companycompany: "competes_with",
    company_company: "competes_with",
    competitor: "competes_with",
    supplier: "supplier_of",
    supplies: "supplier_of",
    customer_of: "supplier_of",
    customer: "supplier_of",
    subsidiary: "subsidiary_of",
    parent_of: "subsidiary_of",
    owned_by: "subsidiary_of",
    regulated_by: "associated_with",
    governs: "associated_with",
    related: "related_to",
  };

  function normalizeEdgeType(value) {
    const normalized = String(value || "related_to").toLowerCase();
    if (!normalized) return "related_to";
    return EDGE_TYPE_ALIASES[normalized] || normalized;
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
        color: node.color || NODE_COLORS[type] || GRAPH_THEME.defaultNode,
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
      type: normalizeEdgeType(edge.type || edge.edge_type || "related_to"),
      confidence: Number.isFinite(Number(edge.confidence)) ? Number(edge.confidence) : 1,
      firstSeen: edge.firstSeen || edge.first_seen || null,
      lastConfirmed: edge.lastConfirmed || edge.last_confirmed || null,
      evidenceCount: Number(edge.evidenceCount || edge.evidence_count || 0),
      sourceName: edge.source_name || edge.source_label || edge.sourceName || null,
    }));

    return { nodes, edges, nodeMap };
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

  function formatActivity(days) {
    if (!Number.isFinite(days)) return "No recent activity window";
    if (days <= 30) return "Active in the last 30 days";
    if (days <= 90) return "Active in the last 90 days";
    if (days <= 365) return "Active in the last year";
    return "Historical / long-tail activity";
  }

  function updateSearchStatus(message, isAlert) {
    const statusEl = document.getElementById("graph-search-status");
    if (!statusEl) return;
    statusEl.textContent = message;
    statusEl.classList.toggle("is-alert", Boolean(isAlert));
  }

  function updateSelectedPanel(node, details) {
    const panel = document.getElementById("graph-selected-node");
    const labelEl = document.getElementById("graph-selected-label");
    const typeEl = document.getElementById("graph-selected-type");
    const metaEl = document.getElementById("graph-selected-meta");
    const activityEl = document.getElementById("graph-selected-activity");
    const degreeEl = document.getElementById("graph-selected-degree");
    const relationsEl = document.getElementById("graph-selected-relations");
    const neighborsEl = document.getElementById("graph-selected-neighbors");
    const linkEl = document.getElementById("graph-selected-link");
    if (!panel || !labelEl || !typeEl) return;
    if (!node) {
      panel.classList.add("hidden");
      if (relationsEl) relationsEl.innerHTML = "";
      if (neighborsEl) neighborsEl.innerHTML = "";
      return;
    }
    const info = details || {};
    const degree = Number(info.degree || 0);
    const neighbors = Array.isArray(info.neighbors) ? info.neighbors : [];
    const relations = Array.isArray(info.relations) ? info.relations : [];
    labelEl.textContent = node.label;
    typeEl.textContent = node.type.charAt(0).toUpperCase() + node.type.slice(1);
    if (metaEl) metaEl.textContent = `${node.id} • ${node.clusterKey}`;
    if (activityEl) activityEl.textContent = formatActivity(Number(node.activityDays || 3650));
    if (degreeEl) degreeEl.textContent = degree === 1 ? "1 direct link" : `${degree} direct links`;
    if (relationsEl) {
      relationsEl.innerHTML = "";
      if (relations.length) {
        relations.forEach((relation) => {
          const pill = document.createElement("span");
          pill.className = "graph-chip";
          pill.textContent = `${relation.label} ${relation.count}`;
          relationsEl.appendChild(pill);
        });
      } else {
        const empty = document.createElement("span");
        empty.className = "text-xs text-muted-foreground";
        empty.textContent = "No visible relationship metadata.";
        relationsEl.appendChild(empty);
      }
    }
    if (neighborsEl) {
      neighborsEl.innerHTML = "";
      if (neighbors.length) {
        neighbors.slice(0, 10).forEach((neighbor) => {
          const pill = document.createElement("span");
          pill.className = "graph-chip";
          pill.textContent = neighbor;
          neighborsEl.appendChild(pill);
        });
      } else {
        const empty = document.createElement("span");
        empty.className = "text-xs text-muted-foreground";
        empty.textContent = "No visible neighbors in the current filter state.";
        neighborsEl.appendChild(empty);
      }
    }
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

  function updateFilterUI(activeType) {
    const activeFilterEl = document.getElementById("graph-active-filter");
    const filterChips = Array.from(document.querySelectorAll(".graph-filter-chip"));
    if (activeFilterEl) activeFilterEl.textContent = "Active: " + (activeType ? "1" : "0");
    filterChips.forEach((chip) => {
      const isActive = chip.getAttribute("data-type") === activeType;
      chip.classList.toggle("apex-chip-active", isActive);
      chip.classList.toggle("apex-chip-idle", !isActive);
      chip.setAttribute("aria-pressed", String(isActive));
    });
  }

  function isUuid(value) {
    return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(String(value || ""));
  }

  function normalizeCatalog(payload, fallbackNodes) {
    const source = Array.isArray(payload && payload.catalog) && payload.catalog.length
      ? payload.catalog
      : (fallbackNodes || []);
    return source.map((node) => ({
      id: String(node.id),
      label: String(node.label || node.id || "unknown"),
      type: normalizeType(node.type || node.node_type),
    }));
  }

  function formatCatalogChoice(node) {
    const shortId = String(node.id).slice(0, 8);
    return `${node.label} [${node.type}] ${shortId}`;
  }

  function parseApiData(payload) {
    if (!payload || typeof payload !== "object") return payload;
    if (Object.prototype.hasOwnProperty.call(payload, "data")) return payload.data;
    return payload;
  }

  function computeGraphAnalytics(nodes, edges) {
    const degreeIndex = {};
    const edgeTypeCounts = {};
    edges.forEach((edge) => {
      degreeIndex[edge.source] = (degreeIndex[edge.source] || 0) + 1;
      degreeIndex[edge.target] = (degreeIndex[edge.target] || 0) + 1;
      edgeTypeCounts[edge.type] = (edgeTypeCounts[edge.type] || 0) + 1;
    });
    const connectors = nodes
      .map((node) => ({
        id: node.id,
        label: node.label,
        degree: degreeIndex[node.id] || 0,
      }))
      .sort((left, right) => right.degree - left.degree || left.label.localeCompare(right.label))
      .slice(0, 4);

    const density = nodes.length <= 1
      ? 0
      : Math.min(1, edges.length / ((nodes.length * (nodes.length - 1)) / 2));

    return {
      nodeCount: nodes.length,
      edgeCount: edges.length,
      density,
      connectors,
      edgeTypeCounts,
    };
  }

  // ── Progressive disclosure ──────────────────────────────────────────────
  // Default view: the selected entity and its first-degree HIGH-CONFIDENCE
  // relations, or (with nothing selected) recent changes only. The expansion
  // controls reveal supplier / customer / people / competitor relations and
  // weak (low-confidence) edges that the default slice hides.
  const HIGH_CONFIDENCE = 0.6;
  const DEFAULT_RECENT_DAYS = 30;
  const FALLBACK_RECENT_DAYS = 90;

  function edgeConfidence(edge) {
    const value = Number(edge && edge.confidence);
    return Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 1;
  }

  function isWeakEdge(edge) {
    return edgeConfidence(edge) < HIGH_CONFIDENCE;
  }

  function edgeRelationshipLabel(edge) {
    return String((edge && edge.type) || "related_to").replace(/_/g, " ");
  }

  function edgeMatchesExpansion(edge, category, focusId, nodeMap) {
    const type = normalizeType(edge.type);
    switch (category) {
      case "suppliers":
        return type === "supplier_of" && (!focusId || edge.target === focusId);
      case "customers":
        return type === "supplier_of" && (!focusId || edge.source === focusId);
      case "people": {
        if (type !== "associated_with") return false;
        const source = nodeMap[edge.source];
        const target = nodeMap[edge.target];
        if (!source || !target) return false;
        return source.type === "person" || target.type === "person";
      }
      case "competitors":
        return type === "competes_with";
      case "weak":
        return isWeakEdge(edge);
      default:
        return false;
    }
  }

  function recentWindowDays(nodes) {
    const recent = nodes.filter((node) => Number(node.activityDays || 3650) <= DEFAULT_RECENT_DAYS).length;
    return recent >= 4 ? DEFAULT_RECENT_DAYS : FALLBACK_RECENT_DAYS;
  }

  function computeDisclosure(graph, focusId, expansions) {
    const enabled = Object.keys(expansions || {}).filter((key) => expansions[key]);
    const recentWindow = recentWindowDays(graph.nodes);
    const baseIds = new Set(
      graph.nodes
        .filter((node) => Number(node.activityDays || 3650) <= recentWindow)
        .map((node) => node.id),
    );
    if (focusId) baseIds.add(focusId);

    const visibleIds = new Set(baseIds);
    graph.edges.forEach((edge) => {
      if (!baseIds.has(edge.source) && !baseIds.has(edge.target)) return;
      const highConfidence = !isWeakEdge(edge);
      const expanded = enabled.some((category) => edgeMatchesExpansion(edge, category, focusId, graph.nodeMap));
      if (!highConfidence && !expanded) return;
      visibleIds.add(edge.source);
      visibleIds.add(edge.target);
    });

    // A graph whose entities all sit outside the recent window must not render
    // empty on first paint — fall back to the full slice.
    let modeLabel;
    if (visibleIds.size < Math.min(4, graph.nodes.length)) {
      graph.nodes.forEach((node) => visibleIds.add(node.id));
      modeLabel = "Overview slice";
    } else if (focusId) {
      const focusNode = graph.nodeMap[focusId];
      modeLabel = `Focus: ${focusNode ? focusNode.label : "selected"} + 1st-degree`;
    } else {
      modeLabel = `Focus: recent changes (${recentWindow}d)`;
    }

    const hint = focusId
      ? `Focused on ${graph.nodeMap[focusId] ? graph.nodeMap[focusId].label : "selected entity"} with first-degree high-confidence relations. Expand the controls to reveal more relation classes.`
      : `Default view shows recent changes (${recentWindow}d) and their high-confidence relations. Select an entity to focus, or expand the controls for suppliers, customers, people, competitors, and weak edges.`;

    return {
      visibleNodes: graph.nodes.filter((node) => visibleIds.has(node.id)),
      modeLabel,
      hint,
    };
  }

  function updateExpansionUI(graph, focusId) {
    const buttons = Array.from(document.querySelectorAll("#graph-expansion-controls [data-expansion]"));
    if (!buttons.length) return;
    const enabled = buttons.filter((button) => button.getAttribute("aria-pressed") === "true");
    buttons.forEach((button) => {
      const category = button.getAttribute("data-expansion");
      const applicable = graph.edges.some((edge) => edgeMatchesExpansion(edge, category, focusId, graph.nodeMap));
      button.disabled = !applicable;
      button.title = applicable ? `Toggle ${category} relations` : `No ${category} relations in this slice`;
    });
    const summary = document.getElementById("graph-expansion-summary");
    if (summary) {
      summary.textContent = enabled.length
        ? `Expanded: ${enabled.map((button) => button.getAttribute("data-expansion")).join(", ")}`
        : "Collapsed";
    }
  }

  function edgeEndpointLabel(graph, id) {
    const node = graph.nodeMap[id];
    return node ? node.label : String(id);
  }

  function showEdgeDetail(graph, edge) {
    const panel = document.getElementById("graph-edge-detail");
    if (!panel) return;
    const set = (id, value) => {
      const el = document.getElementById(id);
      if (el) el.textContent = value;
    };
    set("graph-edge-summary", `${edgeEndpointLabel(graph, edge.source)} → ${edgeEndpointLabel(graph, edge.target)}`);
    set("graph-edge-relationship", edgeRelationshipLabel(edge));
    set("graph-edge-confidence", `${Math.round(edgeConfidence(edge) * 100)}%`);
    set("graph-edge-first-seen", edge.firstSeen || "Unknown");
    set("graph-edge-last-confirmed", edge.lastConfirmed || "Unknown");
    set("graph-edge-evidence", String(Number.isFinite(Number(edge.evidenceCount)) ? Number(edge.evidenceCount) : 0));
    set("graph-edge-source", edge.sourceName || "graph_edges");
    panel.classList.remove("hidden");
  }

  function clearEdgeDetail() {
    const panel = document.getElementById("graph-edge-detail");
    if (panel) panel.classList.add("hidden");
  }

  function updateAnalyticsPanel(state) {
    const modeEl = document.getElementById("graph-analytics-mode");
    const densityEl = document.getElementById("graph-analytics-density");
    const countsEl = document.getElementById("graph-analytics-counts");
    const connectorsEl = document.getElementById("graph-analytics-connectors");
    const pathSummaryEl = document.getElementById("graph-path-summary");
    if (!modeEl || !densityEl || !countsEl || !connectorsEl || !pathSummaryEl) return;

    modeEl.textContent = state.modeLabel || "Overview slice";
    densityEl.textContent = state.analytics ? state.analytics.density.toFixed(2) : "0.00";
    countsEl.textContent = `${state.analytics ? state.analytics.nodeCount : 0} nodes • ${state.analytics ? state.analytics.edgeCount : 0} edges`;
    pathSummaryEl.textContent = state.pathSummary || "No active path query.";

    connectorsEl.innerHTML = "";
    if (state.analytics && state.analytics.connectors.length) {
      state.analytics.connectors.forEach((connector) => {
        const pill = document.createElement("span");
        pill.className = "graph-chip";
        pill.textContent = `${connector.label} ${connector.degree}`;
        connectorsEl.appendChild(pill);
      });
      return;
    }

    const empty = document.createElement("span");
    empty.className = "text-xs text-muted-foreground";
    empty.textContent = "No connector pattern in the current graph slice.";
    connectorsEl.appendChild(empty);
  }

  function init() {
    const container = document.getElementById("graph-container");
    if (!container) return;
    if (container.__graphInitialized) return;
    container.__graphInitialized = true;

    const embeddedRaw = document.getElementById("graph-data") ? document.getElementById("graph-data").textContent : container.dataset.graphJson;
    if (embeddedRaw) {
      try {
        const payload = JSON.parse(embeddedRaw);
        return initWithData(container, payload);
      } catch (error) {
        console.error("[graph] invalid embedded JSON", error);
      }
    }

    container.innerHTML = '<p class="text-muted-foreground text-sm p-4">Could not load graph data.</p>';
  }

  function initWithData(container, payload) {
    const baseGraph = normalizePayload(payload || {});
    let activeGraph = baseGraph;
    const catalog = normalizeCatalog(payload || {}, baseGraph.nodes);
    const catalogById = {};
    const catalogChoiceToId = new Map();
    catalog.forEach((node) => {
      catalogById[node.id] = node;
      catalogChoiceToId.set(formatCatalogChoice(node).toLowerCase(), node.id);
      if (!catalogChoiceToId.has(node.label.toLowerCase())) {
        catalogChoiceToId.set(node.label.toLowerCase(), node.id);
      }
    });

    const layoutCache = new Map();
    let activeType = null;
    let searchText = "";
    let clusterMode = false;
    let activityWindow = "all";
    let selectedId = null;
    const expansions = { suppliers: false, customers: false, people: false, competitors: false, weak: false };
    let viewportState = { scale: 1, panX: 0, panY: 0 };
    let modeLabel = "Overview slice";
    let disclosureLabel = "Overview slice";
    let pathSummary = "No active path query.";
    let loadingRemote = false;

    const filterChips = Array.from(document.querySelectorAll(".graph-filter-chip"));
    const resetBtn = document.getElementById("graph-reset-filters");
    const searchInput = document.getElementById("graph-search-input");
    const activityFilter = document.getElementById("graph-activity-filter");
    const clusterToggle = document.getElementById("graph-cluster-toggle");
    const neighborhoodBtn = document.getElementById("graph-load-neighborhood");
    const pathInput = document.getElementById("graph-path-target");
    const pathDatalist = document.getElementById("graph-node-catalog");
    const pathBtn = document.getElementById("graph-find-path");
    const overviewBtn = document.getElementById("graph-return-overview");
    const modeBadge = document.getElementById("graph-mode-label");
    const exportJsonBtn = document.getElementById("graph-export-json");
    const exportSvgBtn = document.getElementById("graph-export-svg");
    const visibleCountEl = document.getElementById("graph-visible-count");
    const expansionButtons = Array.from(document.querySelectorAll("#graph-expansion-controls [data-expansion]"));

    if (pathDatalist) {
      pathDatalist.innerHTML = "";
      catalog
        .filter((node) => isUuid(node.id))
        .sort((left, right) => left.label.localeCompare(right.label))
        .forEach((node) => {
          const option = document.createElement("option");
          option.value = formatCatalogChoice(node);
          pathDatalist.appendChild(option);
        });
    }

    function currentSelectedNode() {
      return activeGraph.nodeMap[selectedId] || baseGraph.nodeMap[selectedId] || catalogById[selectedId] || null;
    }

    function displayModeLabel() {
      if (modeLabel !== "Overview slice") return modeLabel;
      if (searchText) return "Search results";
      return disclosureLabel;
    }

    function canUseServerGraph(node) {
      return Boolean(node && isUuid(node.id));
    }

    function resolvePathTargetId(value) {
      const normalized = String(value || "").trim();
      if (!normalized) return null;
      if (catalogChoiceToId.has(normalized.toLowerCase())) {
        return catalogChoiceToId.get(normalized.toLowerCase()) || null;
      }
      if (isUuid(normalized) && catalogById[normalized]) {
        return normalized;
      }
      return null;
    }

    function syncRemoteControls() {
      const selectedNode = currentSelectedNode();
      const selectedSupported = canUseServerGraph(selectedNode);
      const targetId = resolvePathTargetId(pathInput ? pathInput.value : "");
      if (neighborhoodBtn) neighborhoodBtn.disabled = !selectedSupported || loadingRemote;
      if (pathBtn) pathBtn.disabled = !selectedSupported || !targetId || targetId === selectedId || loadingRemote;
      if (overviewBtn) overviewBtn.disabled = modeLabel === "Overview slice" || loadingRemote;
      if (modeBadge) modeBadge.textContent = `Mode: ${displayModeLabel().toLowerCase()}`;
    }

    function setActiveGraph(nextPayload, nextModeLabel, nextPathSummary) {
      activeGraph = normalizePayload(nextPayload || {});
      modeLabel = nextModeLabel || "Overview slice";
      pathSummary = nextPathSummary || "No active path query.";
      viewportState = { scale: 1, panX: 0, panY: 0 };
      syncRemoteControls();
      rerender();
    }

    async function fetchGraphData(url) {
      const response = await fetch(url, {
        headers: {
          Accept: "application/json",
        },
        credentials: "same-origin",
      });
      const payload = parseApiData(await response.json());
      if (!response.ok) {
        const message = payload && payload.error && payload.error.message ? payload.error.message : `Request failed (${response.status})`;
        throw new Error(message);
      }
      return payload;
    }

    function payloadFromPathResponse(data) {
      return {
        nodes: (data.path || []).map((step) => {
          const catalogNode = catalogById[step.node_id] || {};
          return {
            id: step.node_id,
            label: step.node_label,
            node_type: catalogNode.type || "company",
            type: catalogNode.type || "company",
            size: 14,
            clusterKey: catalogNode.type || "company",
          };
        }),
        edges: (data.path || []).slice(1).map((step, index) => ({
          source: data.path[index].node_id,
          target: step.node_id,
          edge_type: step.edge_type || "related_to",
          type: step.edge_type || "related_to",
          weight: step.edge_weight || 1,
        })),
        catalog,
      };
    }

    const rerender = function () {
      const graph = activeGraph;
      const graphAdjacency = adjacencyFromEdges(graph.nodes, graph.edges);
      const graphDegreeIndex = {};
      graph.edges.forEach((edge) => {
        graphDegreeIndex[edge.source] = (graphDegreeIndex[edge.source] || 0) + 1;
        graphDegreeIndex[edge.target] = (graphDegreeIndex[edge.target] || 0) + 1;
      });

      const focusId = selectedId && graph.nodeMap[selectedId] ? selectedId : null;
      const disclosure = computeDisclosure(graph, focusId, expansions);
      disclosureLabel = disclosure.modeLabel;

      let nodes = disclosure.visibleNodes;
      let matchedNodes = [];
      let matchedIds = new Set();
      let contextNeighborCount = 0;

      if (activeType) {
        nodes = nodes.filter((node) => node.type === activeType);
      }

      if (activityWindow !== "all") {
        const maxDays = Number(activityWindow);
        nodes = nodes.filter((node) => Number(node.activityDays || 3650) <= maxDays);
      }

      if (searchText) {
        const lowered = searchText.toLowerCase();
        // Searching widens the pool past the collapsed disclosure slice so
        // users can still reach entities that the default view hides.
        const searchBase = activeType ? graph.nodes.filter((node) => node.type === activeType) : graph.nodes;
        matchedNodes = searchBase.filter((node) => {
          const matches = node.label.toLowerCase().includes(lowered) || node.id.toLowerCase().includes(lowered);
          if (matches) matchedIds.add(node.id);
          return matches;
        });
        if (matchedIds.size > 0) {
          const contextIds = new Set(matchedNodes.map((node) => node.id));
          matchedNodes.forEach((node) => {
            (graphAdjacency[node.id] || []).forEach((neighborId) => {
              contextIds.add(neighborId);
            });
          });
          contextNeighborCount = Math.max(0, contextIds.size - matchedIds.size);
          nodes = graph.nodes.filter((node) => contextIds.has(node.id) && (activityWindow === "all" || Number(node.activityDays || 3650) <= Number(activityWindow)));
        } else {
          nodes = [];
        }
      }

      const nodeSet = new Set(nodes.map((node) => node.id));
      const edges = graph.edges.filter((edge) => nodeSet.has(edge.source) && nodeSet.has(edge.target));
      if (visibleCountEl) visibleCountEl.textContent = "Visible: " + nodes.length;
      updateFilterUI(activeType);
      updateExpansionUI(graph, focusId);

      if (selectedId && !nodeSet.has(selectedId)) {
        selectedId = null;
      }

      if (searchText && matchedIds.size === 0) {
        updateSearchStatus("No matches in the current graph view.", true);
      } else if (searchText) {
        updateSearchStatus(`Matched ${matchedIds.size} node${matchedIds.size === 1 ? "" : "s"} and kept ${contextNeighborCount} connected neighbor${contextNeighborCount === 1 ? "" : "s"} for context.`, false);
      } else {
        updateSearchStatus(disclosure.hint, false);
      }

      if (searchText && !selectedId && matchedIds.size > 0) {
        selectedId = matchedNodes
          .slice()
          .sort((left, right) => (graphDegreeIndex[right.id] || 0) - (graphDegreeIndex[left.id] || 0))[0].id;
      }

      updateAnalyticsPanel({
        modeLabel: displayModeLabel(),
        pathSummary,
        analytics: computeGraphAnalytics(nodes, edges),
      });

      render(container, nodes, edges, {
        clusterMode,
        matchIds: matchedIds,
        selectedId,
        viewportState,
        layoutCache,
        onSelectChange: function (nextId) {
          selectedId = nextId;
          syncRemoteControls();
        },
        onViewportChange: function (nextViewport) {
          viewportState = nextViewport;
        },
      });

      syncRemoteControls();
    };

    filterChips.forEach((chip) => {
      chip.addEventListener("click", function () {
        const type = chip.getAttribute("data-type");
        activeType = activeType === type ? null : type;
        updateSelectedPanel(null);
        rerender();
      });
    });

    expansionButtons.forEach((button) => {
      button.addEventListener("click", function () {
        const category = button.getAttribute("data-expansion");
        if (!category || !Object.prototype.hasOwnProperty.call(expansions, category)) return;
        expansions[category] = !expansions[category];
        button.setAttribute("aria-pressed", String(expansions[category]));
        button.classList.toggle("apex-chip-active", expansions[category]);
        button.classList.toggle("apex-chip-idle", !expansions[category]);
        rerender();
      });
    });

    if (resetBtn) {
      resetBtn.addEventListener("click", function () {
        activeType = null;
        searchText = "";
        clusterMode = false;
        activityWindow = "all";
        selectedId = null;
        activeGraph = baseGraph;
        modeLabel = "Overview slice";
        pathSummary = "No active path query.";
        viewportState = { scale: 1, panX: 0, panY: 0 };
        if (searchInput) searchInput.value = "";
        if (activityFilter) activityFilter.value = "all";
        if (pathInput) pathInput.value = "";
        if (clusterToggle) clusterToggle.textContent = "Relationship Layout";
        Object.keys(expansions).forEach((category) => {
          expansions[category] = false;
        });
        expansionButtons.forEach((button) => {
          button.setAttribute("aria-pressed", "false");
          button.classList.remove("apex-chip-active");
          button.classList.add("apex-chip-idle");
        });
        clearEdgeDetail();
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

    if (pathInput) {
      pathInput.addEventListener("input", function () {
        syncRemoteControls();
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
        clusterToggle.textContent = clusterMode ? "Type Layout" : "Relationship Layout";
        rerender();
      });
    }

    if (overviewBtn) {
      overviewBtn.addEventListener("click", function () {
        setActiveGraph(payload, "Overview slice", "No active path query.");
      });
    }

    if (neighborhoodBtn) {
      neighborhoodBtn.addEventListener("click", async function () {
        const selectedNode = currentSelectedNode();
        if (!canUseServerGraph(selectedNode)) return;
        loadingRemote = true;
        syncRemoteControls();
        updateSearchStatus(`Loading neighborhood for ${selectedNode.label}...`, false);
        try {
          const data = await fetchGraphData(`/api/graph/neighborhood/${selectedNode.id}?depth=2&max_nodes=36`);
          setActiveGraph(data, `Neighborhood of ${selectedNode.label}`, "No active path query.");
          selectedId = selectedNode.id;
          updateSearchStatus(`Loaded ${data.total_neighbors || 0} connected nodes around ${selectedNode.label}.`, false);
        } catch (error) {
          updateSearchStatus(error && error.message ? error.message : "Failed to load graph neighborhood.", true);
        } finally {
          loadingRemote = false;
          syncRemoteControls();
        }
      });
    }

    if (pathBtn) {
      pathBtn.addEventListener("click", async function () {
        const selectedNode = currentSelectedNode();
        const targetId = resolvePathTargetId(pathInput ? pathInput.value : "");
        if (!canUseServerGraph(selectedNode) || !targetId || targetId === selectedNode.id) return;
        const targetNode = catalogById[targetId] || { label: targetId };
        loadingRemote = true;
        syncRemoteControls();
        updateSearchStatus(`Finding path from ${selectedNode.label} to ${targetNode.label}...`, false);
        try {
          const data = await fetchGraphData(`/api/graph/path/${selectedNode.id}/${targetId}?max_hops=5`);
          if (!data.found || !Array.isArray(data.path) || data.path.length < 2) {
            pathSummary = `No path found between ${selectedNode.label} and ${targetNode.label} within 5 hops.`;
            updateAnalyticsPanel({
              modeLabel,
              pathSummary,
              analytics: computeGraphAnalytics(activeGraph.nodes, activeGraph.edges),
            });
            updateSearchStatus(pathSummary, true);
            return;
          }

          setActiveGraph(
            payloadFromPathResponse(data),
            `Path ${selectedNode.label} → ${targetNode.label}`,
            `${data.total_hops} hop${data.total_hops === 1 ? "" : "s"} • weight ${Number(data.total_weight || 0).toFixed(2)}`,
          );
          selectedId = selectedNode.id;
          updateSearchStatus(`Resolved path from ${selectedNode.label} to ${targetNode.label}.`, false);
        } catch (error) {
          updateSearchStatus(error && error.message ? error.message : "Failed to resolve graph path.", true);
        } finally {
          loadingRemote = false;
          syncRemoteControls();
        }
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

    syncRemoteControls();
    rerender();
  }

  function render(container, rawNodes, rawEdges, options) {
    if (typeof container.__graphCleanup === "function") container.__graphCleanup();
    const clusterMode = Boolean(options && options.clusterMode);
    const matchIds = options && options.matchIds ? options.matchIds : new Set();
    const layoutCache = options && options.layoutCache ? options.layoutCache : new Map();
    const onSelectChange = options && typeof options.onSelectChange === "function" ? options.onSelectChange : function () {};
    const onViewportChange = options && typeof options.onViewportChange === "function" ? options.onViewportChange : function () {};

    container.innerHTML = "";
    const w = container.clientWidth || 800;
    const h = container.clientHeight || 550;

    container.style.background = [
      "radial-gradient(circle at top left, rgba(255,255,255,0.92), transparent 34%)",
      `linear-gradient(180deg, ${GRAPH_THEME.stageTop} 0%, ${GRAPH_THEME.stageBottom} 100%)`
    ].join(", ");

    let scale = options && options.viewportState ? Number(options.viewportState.scale || 1) : 1;
    let panX = options && options.viewportState ? Number(options.viewportState.panX || 0) : 0;
    let panY = options && options.viewportState ? Number(options.viewportState.panY || 0) : 0;
    let dragging = false;
    let dragNode = null;
    let dragStart = { x: 0, y: 0, px: 0, py: 0 };
    let hoveredId = null;
    let selectedId = options && options.selectedId ? options.selectedId : null;
    let disposed = false;
    let touchMode = null;
    let touchOrigin = null;
    let animationHandle = 0;

    const degreeIndex = {};
    rawEdges.forEach((edge) => {
      degreeIndex[edge.source] = (degreeIndex[edge.source] || 0) + 1;
      degreeIndex[edge.target] = (degreeIndex[edge.target] || 0) + 1;
    });
    const graphAdjacency = adjacencyFromEdges(rawNodes, rawEdges);
    const clusterAnchors = buildClusterAnchors(rawNodes, w, h, clusterMode);
    const relationshipLayout = buildRelationshipLayout(rawNodes, graphAdjacency, degreeIndex, w, h);
    const clusterCounts = rawNodes.reduce((counts, node) => {
      counts[node.clusterKey] = (counts[node.clusterKey] || 0) + 1;
      return counts;
    }, {});

    const nodes = rawNodes.map((node, index) => {
      const angle = (index / Math.max(1, rawNodes.length)) * Math.PI * 2;
      const anchor = clusterAnchors[node.clusterKey] || { x: w / 2, y: h / 2 };
      const topology = relationshipLayout.positions[node.id];
      const orbit = clusterMode ? Math.min(w, h) * 0.06 : Math.min(w, h) * 0.11;
      const cached = !clusterMode && layoutCache.has(node.id) ? layoutCache.get(node.id) : null;
      const offset = seededOffset(node.id, orbit);
      const degree = degreeIndex[node.id] || 0;
      const radius = Math.max(10, Math.round((node.size || 12) * 0.66 + Math.min(8, degree * 0.42)));
      const baseX = clusterMode ? anchor.x + offset.x + Math.cos(angle) * (orbit * 0.25) : topology ? topology.x : (w / 2) + offset.x;
      const baseY = clusterMode ? anchor.y + offset.y + Math.sin(angle) * (orbit * 0.25) : topology ? topology.y : (h / 2) + offset.y;
      return {
        ...node,
        x: cached && Number.isFinite(cached.x) ? cached.x : baseX,
        y: cached && Number.isFinite(cached.y) ? cached.y : baseY,
        vx: 0,
        vy: 0,
        radius,
        degree,
        componentIndex: relationshipLayout.componentByNode[node.id] || 0,
      };
    });

    container.__graphExportData = {
      nodes: rawNodes,
      edges: rawEdges,
      options: { clusterMode },
    };

    const nodeById = {};
    const adjacency = {};
    const relationIndex = {};
    nodes.forEach((node) => {
      nodeById[node.id] = node;
      adjacency[node.id] = [];
      relationIndex[node.id] = {};
    });
    rawEdges.forEach((edge) => {
      if (adjacency[edge.source]) adjacency[edge.source].push(edge.target);
      if (adjacency[edge.target]) adjacency[edge.target].push(edge.source);
      if (relationIndex[edge.source]) relationIndex[edge.source][edge.type] = (relationIndex[edge.source][edge.type] || 0) + 1;
      if (relationIndex[edge.target]) relationIndex[edge.target][edge.type] = (relationIndex[edge.target][edge.type] || 0) + 1;
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
    const backdropG = document.createElementNS(ns, "g");
    const edgeG = document.createElementNS(ns, "g");
    const nodeG = document.createElementNS(ns, "g");
    rootG.appendChild(backdropG);
    rootG.appendChild(edgeG);
    rootG.appendChild(nodeG);
    svg.appendChild(rootG);

    const clusterEls = clusterMode ? Object.keys(clusterAnchors).filter((clusterKey) => clusterCounts[clusterKey]).map((clusterKey) => {
      const anchor = clusterAnchors[clusterKey];
      const group = document.createElementNS(ns, "g");
      const circle = document.createElementNS(ns, "circle");
      circle.setAttribute("cx", String(anchor.x));
      circle.setAttribute("cy", String(anchor.y));
      circle.setAttribute("r", String(anchor.radius + Math.min(80, (clusterCounts[clusterKey] || 1) * 5)));
      circle.setAttribute("fill", NODE_COLORS[clusterKey] || GRAPH_THEME.defaultNode);
      circle.setAttribute("opacity", clusterMode ? "0.14" : "0.08");
      circle.setAttribute("stroke", NODE_COLORS[clusterKey] || GRAPH_THEME.defaultNode);
      circle.setAttribute("stroke-width", "1.5");
      circle.setAttribute("stroke-dasharray", "4 8");
      const label = document.createElementNS(ns, "text");
      label.setAttribute("x", String(anchor.x));
      label.setAttribute("y", String(anchor.y - (anchor.radius + 18)));
      label.setAttribute("text-anchor", "middle");
      label.setAttribute("font-size", "10");
      label.setAttribute("font-weight", "800");
      label.setAttribute("fill", GRAPH_THEME.subduedLabel);
      label.textContent = `${clusterKey.toUpperCase()} ${clusterCounts[clusterKey]}`;
      group.appendChild(circle);
      group.appendChild(label);
      backdropG.appendChild(group);
      return { clusterKey, circle, label };
    }) : [];

    const edgeEls = rawEdges.map((edge) => {
      const line = document.createElementNS(ns, "line");
      const style = EDGE_STYLES[edge.type] || EDGE_STYLES.related_to;
      line.setAttribute("stroke", style.color);
      line.setAttribute("stroke-width", String(Math.max(1, edge.weight * 0.5)));
      line.setAttribute("opacity", "0.45");
      line.setAttribute("class", "graph-edge");
      line.setAttribute("tabindex", "0");
      line.setAttribute("role", "button");
      line.setAttribute("aria-label", `Relationship ${edgeRelationshipLabel(edge)} between ${edgeEndpointLabel({ nodeMap: nodeById }, edge.source)} and ${edgeEndpointLabel({ nodeMap: nodeById }, edge.target)}, confidence ${Math.round(edgeConfidence(edge) * 100)} percent`);
      if (style.dash) line.setAttribute("stroke-dasharray", style.dash);
      line.addEventListener("mouseenter", () => showEdgeDetail({ nodeMap: nodeById }, edge));
      line.addEventListener("focus", () => showEdgeDetail({ nodeMap: nodeById }, edge));
      line.addEventListener("click", (event) => {
        event.stopPropagation();
        showEdgeDetail({ nodeMap: nodeById }, edge);
      });
      line.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          showEdgeDetail({ nodeMap: nodeById }, edge);
        }
      });
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

      const halo = document.createElementNS(ns, "circle");
      halo.setAttribute("r", String(node.radius * 1.7));
      halo.setAttribute("fill", node.color || NODE_COLORS[node.type] || GRAPH_THEME.defaultNode);
      halo.setAttribute("opacity", "0.05");
      group.appendChild(halo);

      const circle = document.createElementNS(ns, "circle");
      const baseRadius = node.radius;
      circle.setAttribute("r", String(baseRadius));
      circle.setAttribute("fill", node.color || NODE_COLORS[node.type] || GRAPH_THEME.defaultNode);
      circle.setAttribute("stroke", "transparent");
      circle.setAttribute("stroke-width", "2");
      circle.style.transition = "r 0.15s, opacity 0.15s";
      group.appendChild(circle);

      const label = document.createElementNS(ns, "text");
      label.setAttribute("text-anchor", "middle");
      label.setAttribute("fill", "currentColor");
      label.setAttribute("font-size", "11");
      label.setAttribute("font-weight", "700");
      label.setAttribute("class", "uppercase");
      label.style.pointerEvents = "none";
      label.style.userSelect = "none";
      label.setAttribute("stroke", "rgba(255,255,255,0.92)");
      label.setAttribute("stroke-width", "3");
      label.setAttribute("paint-order", "stroke");
      label.textContent = node.label.length > 16 ? node.label.slice(0, 15) + "…" : node.label;
      label.classList.add("graph-node-label");
      group.appendChild(label);

      group.addEventListener("mouseenter", () => setHover(node.id));
      group.addEventListener("mouseleave", () => setHover(null));
      group.addEventListener("focus", () => setHover(node.id));
      group.addEventListener("blur", () => setHover(null));
      group.addEventListener("click", () => selectNode(node.id, true));
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
      return { g: group, halo, circle, label, data: node, baseRadius };
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
    badge.className = "graph-viewport-badge";
    badge.textContent = "100%";
    container.appendChild(badge);

    const legend = document.createElement("div");
    legend.className = "graph-legend";
    Object.entries(NODE_COLORS).forEach(([type, color]) => {
      if (!nodes.some((node) => node.type === type)) return;
      const item = document.createElement("div");
      item.className = "graph-legend-item";
      item.innerHTML = `<span class="graph-legend-dot" style="background:${color}"></span><span>${type.replace(/_/g, " ")}</span>`;
      legend.appendChild(item);
    });
    container.appendChild(legend);

    const modeBadge = document.createElement("div");
    modeBadge.className = "graph-mode-badge";
    modeBadge.textContent = clusterMode ? "Type layout" : "Relationship layout";
    container.appendChild(modeBadge);

    function paintFrame() {
      nodeEls.forEach(({ g, data, label, baseRadius }) => {
        g.setAttribute("transform", `translate(${data.x},${data.y})`);
        label.setAttribute("y", String(baseRadius + 13));
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
    }

    function queueSimulation() {
      if (disposed || animationHandle) return;
      animationHandle = requestAnimationFrame(simulate);
    }

    function setHover(id) {
      hoveredId = id;
      updateVisuals();
      if (hoveredId && nodeById[hoveredId]) {
        updateSelectedPanel(nodeById[hoveredId], nodeDetails(hoveredId));
      } else if (selectedId && nodeById[selectedId]) {
        updateSelectedPanel(nodeById[selectedId], nodeDetails(selectedId));
      } else {
        updateSelectedPanel(null);
      }
    }

    function nodeDetails(id) {
      const neighbors = (adjacency[id] || [])
        .filter((neighborId) => nodeById[neighborId])
        .sort((leftId, rightId) => (degreeIndex[rightId] || 0) - (degreeIndex[leftId] || 0))
        .map((neighborId) => nodeById[neighborId].label);
      const relations = Object.entries(relationIndex[id] || {})
        .sort((left, right) => right[1] - left[1])
        .slice(0, 6)
        .map(([label, count]) => ({ label: label.replace(/_/g, " "), count }));
      return {
        degree: neighbors.length,
        neighbors,
        relations,
      };
    }

    function focusNode(id) {
      const node = nodeById[id];
      if (!node) return;
      scale = Math.max(scale, 1.15);
      panX = w / 2 - node.x * scale;
      panY = h / 2 - node.y * scale;
      applyTransform();
    }

    function selectNode(id, focus) {
      selectedId = id;
      onSelectChange(selectedId);
      updateVisuals();
      if (selectedId && nodeById[selectedId]) {
        if (focus) focusNode(selectedId);
        updateSelectedPanel(nodeById[selectedId], nodeDetails(selectedId));
      } else if (hoveredId && nodeById[hoveredId]) {
        updateSelectedPanel(nodeById[hoveredId], nodeDetails(hoveredId));
      } else {
        updateSelectedPanel(null);
      }
    }

    function updateVisuals() {
      const activeId = hoveredId || selectedId;
      const litIds = new Set();
      const activeCluster = activeId && nodeById[activeId] ? nodeById[activeId].clusterKey : null;
      const activeComponent = activeId && nodeById[activeId] ? nodeById[activeId].componentIndex : null;
      if (activeId) {
        litIds.add(activeId);
        (adjacency[activeId] || []).forEach((neighborId) => litIds.add(neighborId));
      }

      clusterEls.forEach(({ clusterKey, circle, label }) => {
        const isActiveCluster = activeCluster && clusterKey === activeCluster;
        circle.setAttribute("opacity", activeCluster ? isActiveCluster ? (clusterMode ? "0.22" : "0.16") : "0.035" : clusterMode ? "0.14" : "0.08");
        label.setAttribute("opacity", activeCluster ? isActiveCluster ? "1" : "0.3" : "0.72");
      });

      nodeEls.forEach(({ halo, circle, label, data, baseRadius }) => {
        const isHovered = hoveredId === data.id;
        const isSelected = selectedId === data.id;
        const isMatched = matchIds.has(data.id);
        const isNeighbor = activeId && litIds.has(data.id) && !isHovered && !isSelected;
        const sameComponent = activeComponent !== null && data.componentIndex === activeComponent;
        const fadeNode = (activeId || matchIds.size > 0) && !litIds.has(data.id) && !isMatched;
        halo.setAttribute("r", String(isSelected ? baseRadius * 2.9 : isHovered ? baseRadius * 2.45 : isMatched ? baseRadius * 2.2 : isNeighbor ? baseRadius * 1.8 : baseRadius * 1.45));
        halo.setAttribute("opacity", isSelected ? "0.24" : isHovered ? "0.18" : isMatched ? "0.16" : isNeighbor ? "0.10" : fadeNode ? "0" : sameComponent ? "0.06" : data.degree >= 4 ? "0.08" : "0.04");
        circle.setAttribute("r", String(isHovered || isSelected ? baseRadius * 1.28 : isNeighbor ? baseRadius * 1.12 : baseRadius));
        circle.setAttribute("stroke", isSelected ? GRAPH_THEME.selectedStroke : isHovered ? GRAPH_THEME.hoverStroke : isMatched ? GRAPH_THEME.matchStroke : "transparent");
        circle.setAttribute("stroke-width", isSelected ? "3" : isHovered || isMatched ? "2.5" : "2");
        circle.setAttribute("opacity", fadeNode ? (sameComponent ? "0.34" : "0.16") : isMatched || isSelected ? "1" : "0.98");
        label.textContent = (isSelected || isHovered || isMatched || scale >= 1.22) && data.label.length > 22 ? data.label.slice(0, 21) + "…" : data.label.length > 16 ? data.label.slice(0, 15) + "…" : data.label;
        label.style.opacity = isSelected || isHovered || isNeighbor || isMatched || scale >= 1.22 ? "1" : fadeNode ? (sameComponent ? "0.22" : "0.10") : scale >= 0.9 ? "0.62" : "0";
        label.setAttribute("font-weight", isSelected ? "900" : isNeighbor ? "800" : "700");
      });

      edgeEls.forEach(({ el, data }) => {
        const style = EDGE_STYLES[data.type] || EDGE_STYLES.related_to;
        const lit = activeId && (data.source === activeId || data.target === activeId);
        const adjacent = activeId && litIds.has(data.source) && litIds.has(data.target);
        el.setAttribute("stroke", lit ? GRAPH_THEME.selectedStroke : style.color);
        el.setAttribute("opacity", activeId ? lit ? "0.92" : adjacent ? "0.38" : "0.045" : matchIds.size > 0 ? matchIds.has(data.source) || matchIds.has(data.target) ? "0.58" : "0.14" : "0.46");
      });
    }

    function applyTransform() {
      rootG.setAttribute("transform", `translate(${panX},${panY}) scale(${scale})`);
      badge.textContent = Math.round(scale * 100) + "%";
      onViewportChange({ scale, panX, panY });
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
        paintFrame();
        queueSimulation();
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
      queueSimulation();
    };

    function distanceBetweenTouches(touches) {
      const dx = touches[0].clientX - touches[1].clientX;
      const dy = touches[0].clientY - touches[1].clientY;
      return Math.sqrt(dx * dx + dy * dy) || 1;
    }

    function midpointBetweenTouches(touches, rect) {
      return {
        x: ((touches[0].clientX + touches[1].clientX) / 2) - rect.left,
        y: ((touches[0].clientY + touches[1].clientY) / 2) - rect.top,
      };
    }

    svg.addEventListener("touchstart", (event) => {
      if (event.touches.length === 1) {
        touchMode = "pan";
        touchOrigin = {
          x: event.touches[0].clientX,
          y: event.touches[0].clientY,
          panX,
          panY,
        };
      } else if (event.touches.length === 2) {
        const rect = svg.getBoundingClientRect();
        touchMode = "pinch";
        touchOrigin = {
          distance: distanceBetweenTouches(event.touches),
          scale,
          midpoint: midpointBetweenTouches(event.touches, rect),
          panX,
          panY,
        };
      }
    }, { passive: true });

    svg.addEventListener("touchmove", (event) => {
      if (!touchMode || !touchOrigin) return;
      if (touchMode === "pan" && event.touches.length === 1) {
        event.preventDefault();
        panX = touchOrigin.panX + (event.touches[0].clientX - touchOrigin.x);
        panY = touchOrigin.panY + (event.touches[0].clientY - touchOrigin.y);
        applyTransform();
      } else if (touchMode === "pinch" && event.touches.length === 2) {
        event.preventDefault();
        const rect = svg.getBoundingClientRect();
        const midpoint = midpointBetweenTouches(event.touches, rect);
        const currentDistance = distanceBetweenTouches(event.touches);
        const nextScale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, touchOrigin.scale * (currentDistance / touchOrigin.distance)));
        const ratio = nextScale / scale;
        panX = midpoint.x - (midpoint.x - panX) * ratio;
        panY = midpoint.y - (midpoint.y - panY) * ratio;
        scale = nextScale;
        applyTransform();
      }
    }, { passive: false });

    svg.addEventListener("touchend", (event) => {
      if (!event.touches || event.touches.length === 0) {
        touchMode = null;
        touchOrigin = null;
      }
    });

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);

    let frame = 0;

    function simulate() {
      animationHandle = 0;
      if (disposed) return;

      nodes.forEach((node) => {
        const anchor = clusterMode
          ? (clusterAnchors[node.clusterKey] || { x: w / 2, y: h / 2 })
          : (relationshipLayout.anchors[node.componentIndex] || { x: w / 2, y: h / 2 });
        const anchorPull = clusterMode ? 0.0044 : 0.00085;
        node.vx += (anchor.x - node.x) * anchorPull;
        node.vy += (anchor.y - node.y) * anchorPull;
      });

      for (let leftIndex = 0; leftIndex < nodes.length; leftIndex += 1) {
        const node = nodes[leftIndex];
        for (let rightIndex = leftIndex + 1; rightIndex < nodes.length; rightIndex += 1) {
          const other = nodes[rightIndex];
          const dx = node.x - other.x;
          const dy = node.y - other.y;
          const dist = Math.sqrt(dx * dx + dy * dy) || 1;
          const sameCluster = node.clusterKey === other.clusterKey;
          const repulsion = sameCluster ? 3600 : 9200;
          const force = repulsion / (dist * dist);
          const fx = (dx / dist) * force * 0.012;
          const fy = (dy / dist) * force * 0.012;
          node.vx += fx;
          node.vy += fy;
          other.vx -= fx;
          other.vy -= fy;

          const minimumDistance = node.radius + other.radius + (sameCluster ? 20 : 34);
          if (dist < minimumDistance) {
            const push = (minimumDistance - dist) * 0.018;
            const px = (dx / dist) * push;
            const py = (dy / dist) * push;
            node.vx += px;
            node.vy += py;
            other.vx -= px;
            other.vy -= py;
          }
        }
      }

      rawEdges.forEach((edge) => {
        const source = nodeById[edge.source];
        const target = nodeById[edge.target];
        if (!source || !target) return;
        const dx = target.x - source.x;
        const dy = target.y - source.y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 1;
        const force = (dist - edgeTargetDistance(edge, source.clusterKey === target.clusterKey)) * 0.0018 * Math.sqrt(edge.weight || 1);
        source.vx += (dx / dist) * force;
        source.vy += (dy / dist) * force;
        target.vx -= (dx / dist) * force;
        target.vy -= (dy / dist) * force;
      });

      nodes.forEach((node) => {
        if (dragNode === node.id) return;
        node.x += node.vx;
        node.y += node.vy;
        if (node.x < 28 || node.x > w - 28) node.vx *= -0.35;
        if (node.y < 28 || node.y > h - 28) node.vy *= -0.35;
        node.x = Math.max(28, Math.min(w - 28, node.x));
        node.y = Math.max(28, Math.min(h - 28, node.y));
        node.vx *= hoveredId === node.id || selectedId === node.id ? 0.82 : 0.9;
        node.vy *= hoveredId === node.id || selectedId === node.id ? 0.82 : 0.9;
        if (!clusterMode) {
          layoutCache.set(node.id, { x: node.x, y: node.y });
        }
      });

      paintFrame();

      frame += 1;
      const kineticEnergy = nodes.reduce((sum, node) => sum + Math.abs(node.vx) + Math.abs(node.vy), 0);
      if (frame < 900 || kineticEnergy > 4.8 || dragging) {
        queueSimulation();
      }
    }

    applyTransform();
    paintFrame();
    updateVisuals();
    if (selectedId && nodeById[selectedId]) {
      if (matchIds.has(selectedId)) focusNode(selectedId);
      updateSelectedPanel(nodeById[selectedId], nodeDetails(selectedId));
    }
    queueSimulation();

    container.__graphCleanup = function () {
      disposed = true;
      if (animationHandle) cancelAnimationFrame(animationHandle);
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

  document.addEventListener("htmx:afterSwap", function (event) {
    const target = event && event.detail ? event.detail.target : null;
    if (!target) return;
    if (target.id === "main-results" || target.querySelector("#graph-container")) {
      init();
    }
  });
})();