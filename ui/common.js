// Shared helpers for the three windows. Plain DOM, no framework.

export const invoke = window.__TAURI__.core.invoke;

/** Calls `render` with the current snapshot now and on every update from the app. */
export function onSnapshot(render) {
  invoke("get_snapshot").then(render);
  window.__TAURI__.event.listen("snapshot", (e) => render(e.payload));
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
  const svg = s("svg", { viewBox: `0 0 ${width} ${height}`, width, height, class: "spark" });
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
  svg.append(s("polyline", { points: pts.map((q) => `${x(q.t).toFixed(1)},${y(q.v).toFixed(1)}`).join(" "), class: "line" }));
  const last = pts[pts.length - 1];
  svg.append(s("circle", { cx: x(last.t), cy: y(last.v), r: 3.2, class: "dot" }));
  return svg;
}
