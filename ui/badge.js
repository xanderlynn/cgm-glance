import { onSnapshot, h, displayValue, sparkline } from "./common.js";

// Window label is "badge-all" (combined) or "badge-<person id>".
const label = window.__TAURI__.window.getCurrentWindow().label;
const root = document.getElementById("root");
const SIZE = { s: 0.8, m: 1, l: 1.3 };
const fresh = (p) => ["in_range", "high", "low", "urgent_low"].includes(p.status);
const shortAge = (p) => (p.age_min == null ? "" : p.age_min === 0 ? "now" : `${p.age_min}m`);

function badge(p, style, k, showInitials) {
  const arrow = fresh(p) ? p.arrow : "";
  const tag = [showInitials && p.initials, arrow].filter(Boolean).join(" ");
  const cls = `b ${style} st-${p.status}${p.is_demo ? " demo" : ""}`;
  const title = `${p.name}${p.is_demo ? " (demo data)" : ""}`;
  const v = displayValue(p);
  const el = {
    // Color only. The arrow stays as a shape cue so color is never the only signal.
    dot: () => h("div", {}, p.status === "sign_in" ? "!" : arrow),
    number: () => [h("div", { class: "n" }, v), tag && h("div", { class: "t" }, tag)],
    arrow: () => [h("div", { class: "n" }, `${v} ${arrow}`.trim()), showInitials && h("div", { class: "t" }, p.initials)],
    age: () => [
      h("span", { class: "d" }),
      h("span", { class: "n" }, `${v} ${arrow}`.trim()),
      h("span", { class: "t" }, [showInitials && p.initials, shortAge(p), p.is_demo && "demo"].filter(Boolean).join(" · ")),
    ],
    card: () => [
      h("div", { class: "t" }, [p.name, shortAge(p)].filter(Boolean).join(" · ")),
      h("div", { class: "n" }, `${v} ${arrow}`.trim()),
      sparkline(p, Math.round(214 * k), Math.round(34 * k)),
    ],
  }[style]();
  return h("div", { class: cls, style: `--k:${k}`, title }, el);
}

onSnapshot((snap) => {
  const b = snap.settings.badge;
  const k = SIZE[b.size] ?? 1;
  const chosen = snap.people.filter((p) => b.people.length === 0 || b.people.includes(p.id));
  const shown = label === "badge-all" ? chosen : chosen.filter((p) => `badge-${p.id}` === label);
  const several = snap.people.length > 1;
  document.body.style.opacity = b.opacity;
  // Dragging is allowed only in free position mode and when not locked.
  const drag = !b.lock && b.position === "free";
  root.classList.toggle("drag", drag);
  if (drag) root.setAttribute("data-tauri-drag-region", "");
  else root.removeAttribute("data-tauri-drag-region");
  root.replaceChildren(...shown.map((p) => badge(p, b.style, k, several)));
});
