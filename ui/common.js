// Shared helpers for the three windows. Plain DOM, no framework.

export const invoke = window.__TAURI__.core.invoke;

/** Calls `render` with the current snapshot now and whenever the app says it changed. */
export function onSnapshot(render) {
  // Updates arrive as a tiny "changed" signal; the data comes back through invoke,
  // which avoids WebKit's per-script memory growth for large event payloads.
  // A window that can't be seen (the hidden popup, a minimized Settings) skips updates
  // and catches up once when it's shown again.
  let pending = false;
  let missed = false;
  const pull = async () => {
    if (document.hidden) {
      missed = true;
      return;
    }
    if (pending) return;
    pending = true;
    try {
      render(await invoke("get_snapshot"));
    } finally {
      pending = false;
    }
  };
  pull();
  window.__TAURI__.event.listen("changed", pull);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && missed) {
      missed = false;
      pull();
    }
  });
}

/** Builds an element. Text always goes in as textContent, never as HTML. */
export function h(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k === "class") node.className = v;
    else if (k === "style") node.style.cssText = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return node;
}

export function ageText(min) {
  if (min == null) return "";
  if (min === 0) return "just now";
  if (min < 60) return `${min} min ago`;
  return `${Math.floor(min / 60)} h ago`;
}

/** The number to show, or a placeholder that can't be mistaken for a reading. */
export function displayValue(p) {
  if (p.status === "sign_in") return "!";
  return p.value ?? "---";
}

export const STATUS_WORD = {
  in_range: null,
  high: "High",
  low: "Low",
  urgent_low: "URGENT LOW",
  stale: "Old reading",
  no_data: "No recent reading",
  sign_in: "Sign in needed",
  error: "Can't connect",
  loading: "Loading",
};

/** A small colored label like "Low" or "Old reading". Null when in range. */
export function statusChip(p) {
  const word = STATUS_WORD[p.status];
  return word ? h("span", { class: `chip st-${p.status}` }, word) : null;
}

export function avatar(p, size = 32) {
  return h("div", { class: "avatar", style: `background:${p.color};width:${size}px;height:${size}px` }, p.initials);
}

const SVG = "http://www.w3.org/2000/svg";
function s(tag, attrs) {
  const n = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
  return n;
}

/** Last 3 hours with the target range shaded. */
export function sparkline(p, width, height, { band = true } = {}) {
  // Stretches to its container's width so it can never push a card wider than the window.
  const svg = s("svg", { viewBox: `0 0 ${width} ${height}`, width: "100%", height, preserveAspectRatio: "none", class: "spark" });
  const pts = p.history;
  if (!pts.length) return svg;
  const now = Date.now();
  const t0 = now - 3 * 3600_000;
  const lo = Math.min(p.low * 0.6, ...pts.map((q) => q.v));
  const hi = Math.max(p.high * 1.4, ...pts.map((q) => q.v));
  const x = (t) => ((t - t0) / (now - t0)) * (width - 6) + 3;
  const y = (v) => height - 3 - ((v - lo) / (hi - lo)) * (height - 6);
  if (band) {
    svg.append(s("rect", { x: 0, y: y(p.high), width, height: y(p.low) - y(p.high), class: "band" }));
  }
  svg.append(s("polyline", { points: pts.map((q) => `${x(q.t).toFixed(1)},${y(q.v).toFixed(1)}`).join(" "), class: "line", "vector-effect": "non-scaling-stroke" }));
  const last = pts[pts.length - 1];
  // A zero-length round-capped line stays a circle when the chart is stretched.
  const ex = x(last.t).toFixed(1), ey = y(last.v).toFixed(1);
  svg.append(s("line", { x1: ex, y1: ey, x2: ex, y2: ey, class: "end", "vector-effect": "non-scaling-stroke" }));
  return svg;
}
