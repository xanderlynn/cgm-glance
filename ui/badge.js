import { onSnapshot, h, displayValue, sparkline, STATUS_WORD } from "./common.js";

// Window label is "badge-all" (combined) or "badge-<person id>".
const label = window.__TAURI__.window.getCurrentWindow().label;
const root = document.getElementById("root");
const SIZE = { s: 0.8, m: 1, l: 1.3 };
const fresh = (p) => ["in_range", "high", "low", "urgent_low"].includes(p.status);
const shortAge = (p) => (p.age_min == null ? "" : p.age_min === 0 ? "now" : `${p.age_min}m`);

// One symbol per state for the color-only style, so color is never the only cue.
function dotSymbol(p) {
  if (p.status === "sign_in") return "!";
  if (p.status === "stale") return "…";
  if (p.status === "high") return "H";
  if (p.status === "low" || p.status === "urgent_low") return "L";
  if (fresh(p)) return p.arrow || "•";
  return "–";
}

function badge(p, style, k, several) {
  const arrow = fresh(p) ? p.arrow : "";
  const v = displayValue(p);
  const age = shortAge(p);
  const word = STATUS_WORD[p.status];
  // Small second line: who, plus the trend, or how old the reading is when it isn't fresh.
  const tag = [several && p.initials, fresh(p) ? arrow : age].filter(Boolean).join(" ");
  const cls = `b ${style} st-${p.status}${p.is_demo ? " demo" : ""}`;
  const title = [p.name, word, p.is_demo && "demo data"].filter(Boolean).join(" · ");
  const el = {
    dot: () => {
      // Out of range shows H/L big, so the trend moves to the small line.
      const small = [several && p.initials, ["high", "low", "urgent_low"].includes(p.status) && arrow].filter(Boolean).join(" ");
      return [h("div", {}, dotSymbol(p)), small && h("div", { class: "t" }, small)];
    },
    number: () => [h("div", { class: "n" }, v), tag && h("div", { class: "t" }, tag)],
    arrow: () => [h("div", { class: "n" }, `${v} ${arrow}`.trim()), h("div", { class: "t" }, [several && p.initials, !fresh(p) && age].filter(Boolean).join(" ") || " ")],
    age: () => [h("span", { class: "d" }), h("span", { class: "n" }, `${v} ${arrow}`.trim()), h("span", { class: "t" }, [several && p.initials, age].filter(Boolean).join(" · "))],
    card: () => [
      h("div", { class: "t" }, [p.name, age, word].filter(Boolean).join(" · ")),
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
