import { invoke, onSnapshot, h, ageText, displayValue, avatar, sparkline } from "./common.js";

let snap = null;
const list = document.getElementById("list");

function card(p) {
  const fresh = ["in_range", "high", "low", "urgent_low"].includes(p.status);
  const sub = [p.unit_label, fresh && p.arrow, fresh && p.delta].filter(Boolean).join("  ");
  const action =
    p.status === "sign_in"
      ? h("button", { onclick: () => invoke("open_settings", { tab: "people" }) }, "Sign in again")
      : p.status === "error"
        ? h("button", { onclick: () => invoke("retry_person", { id: p.id }) }, "Retry now")
        : null;
  return h(
    "div",
    { class: `card st-${p.status}` },
    h(
      "div",
      { class: "row" },
      avatar(p),
      h(
        "div",
        { class: "grow" },
        h("div", {}, h("strong", {}, p.name), " ", p.is_demo && h("span", { class: "demo-pill" }, "DEMO")),
        h("div", { class: "small muted" }, [p.source_label, ageText(p.age_min)].filter(Boolean).join(" · ")),
      ),
      h("div", {}, h("div", { class: "value" }, displayValue(p)), h("div", { class: "small muted sub" }, sub)),
    ),
    p.history.length > 0 && sparkline(p, 330, 54),
    p.message && h("div", { class: "msg" }, p.message),
    action,
  );
}

onSnapshot((s) => {
  snap = s;
  document.getElementById("demo").hidden = !s.demo_mode;
  document.getElementById("badge").textContent = s.settings.badge.enabled ? "Hide desktop badge" : "Show desktop badge";
  list.replaceChildren(
    ...(s.people.length
      ? s.people.map(card)
      : [h("div", { class: "empty" }, "No one added yet.", h("br"), "Add a person to see their readings here.")]),
  );
});

document.getElementById("settings").onclick = () => invoke("open_settings", {});
document.getElementById("add").onclick = () => invoke("open_settings", { tab: "add" });
document.getElementById("quit").onclick = () => invoke("quit_app");
document.getElementById("badge").onclick = () => {
  if (!snap) return;
  const settings = structuredClone(snap.settings);
  settings.badge.enabled = !settings.badge.enabled;
  invoke("save_settings", { settings });
};
document.addEventListener("keydown", (e) => e.key === "Escape" && invoke("hide_popup"));
