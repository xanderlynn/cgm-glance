import { onSnapshot, h, displayValue, STATUS_WORD } from "./common.js";

// Window label is "badge-all" (combined) or "badge-<person id>".
const label = window.__TAURI__.window.getCurrentWindow().label;
const root = document.getElementById("root");
const SIZE = { s: 0.8, m: 1, l: 1.3 };
const fresh = (p) => ["in_range", "high", "low", "urgent_low"].includes(p.status);
const shortAge = (p) => (p.age_min == null ? "" : p.age_min === 0 ? "now" : `${p.age_min}m`);

// Faces style: one face per state. Urgent low and old readings must never look calm.
const FACE = {
  in_range: "🙂",
  high: "😬",
  low: "😟",
  urgent_low: "😱",
  stale: "😴",
  no_data: "😶",
  error: "😶",
  sign_in: "❗",
  loading: "⏳",
};

function badge(p, style, k, several) {
  const arrow = fresh(p) ? p.arrow : "";
  const v = displayValue(p);
  const age = shortAge(p);
  const who = several && p.initials;
  // Second line: the trend when the reading is fresh, otherwise how old it is.
  const extra = fresh(p) ? arrow : age;
  const cls = `b ${style} st-${p.status}${p.is_demo ? " demo" : ""}`;
  // Hovering always shows the full reading, even for the faces style.
  const title = [p.name, p.value && `${p.value} ${p.unit_label} ${arrow}`.trim(), STATUS_WORD[p.status], p.is_demo && "demo data"]
    .filter(Boolean)
    .join(" · ");
  const el = {
    pill: () => [
      h("span", { class: "d" }),
      who && h("span", { class: "t" }, who),
      h("span", { class: "n" }, `${v} ${arrow}`.trim()),
      !fresh(p) && age && h("span", { class: "t" }, age),
    ],
    ring: () => [h("div", { class: "n" }, v), h("div", { class: "t" }, [who, extra].filter(Boolean).join(" ") || " ")],
    faces: () => [h("div", { class: "e" }, FACE[p.status] ?? "😶"), h("div", { class: "t" }, [who, extra].filter(Boolean).join(" ") || " ")],
  }[style] ?? (() => []);
  return h("div", { class: cls, style: `--k:${k}`, title }, el());
}

let shownKey = "";

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
  // Rebuild only when something the badge shows changed.
  const key = JSON.stringify([b.style, b.size, several, shown.map((p) => [p.name, p.initials, p.status, p.value, p.unit_label, p.arrow, p.age_min, p.is_demo])]);
  if (key === shownKey) return;
  shownKey = key;
  root.replaceChildren(...shown.map((p) => badge(p, b.style, k, several)));
});
