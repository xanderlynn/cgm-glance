import { invoke, onSnapshot, h, ageText, displayValue, avatar, sparkline, statusChip } from "./common.js";

let snap = null;
const list = document.getElementById("list");

function card(p) {
  const fresh = ["in_range", "high", "low", "urgent_low"].includes(p.status);
  const sub = p.value ? [p.unit_label, fresh && p.arrow, fresh && p.delta].filter(Boolean).join("  ") : "";
  const action =
    p.status === "sign_in"
      ? h("button", { onclick: () => invoke("open_settings", { tab: `signin:${p.id}` }) }, "Sign in again")
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
        h(
          "div",
          {},
          h("strong", {}, p.name),
          " ",
          p.is_demo && h("span", { class: "demo-pill" }, "DEMO"),
          " ",
          statusChip(p),
          snap.settings.alarms.enabled && snap.settings.alarms.off_for.includes(p.id) && h("span", { class: "chip muted", title: "Alarms are off for this person" }, " alarms off"),
        ),
        h("div", { class: "small muted" }, [p.source_label, ageText(p.age_min)].filter(Boolean).join(" · ")),
      ),
      h("div", {}, h("div", { class: "value" }, displayValue(p)), h("div", { class: "small muted sub" }, sub)),
    ),
    p.alarm && alarmBanner(p),
    p.history.length > 0 && sparkline(p, 330, 40),
    p.message && h("div", { class: "msg" }, p.message),
    action,
  );
}

const clock = (t) => new Date(t).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

function alarmBanner(p) {
  const a = p.alarm;
  const what = a.kind === "low" ? `Low alarm: at or below ${a.threshold}` : `High alarm: at or above ${a.threshold}`;
  return h(
    "div",
    { class: `alarm${a.acked ? " acked" : ""}`, role: a.acked ? null : "alert" },
    h("span", { class: "grow" }, `${what} ${p.unit_label}`, a.acked ? " · acknowledged" : a.snoozed_until ? ` · snoozed until ${clock(a.snoozed_until)}` : ""),
    !a.acked && !a.snoozed_until && h("button", { onclick: () => invoke("snooze_alarms", { id: p.id }) }, "Snooze"),
    !a.acked && h("button", { onclick: () => invoke("acknowledge_alarms", { id: p.id }) }, "Acknowledge"),
  );
}

function showQuiet(s) {
  const q = s.quiet;
  const el = document.getElementById("quiet");
  el.hidden = false;
  const quietBtn = !q?.manual_until && h("button", { onclick: () => invoke("set_quiet", { minutes: 60 }) }, "Quiet for 1 hour");
  const soundErr = !!s.sound_error && h("div", { class: "error small", style: "width:100%" }, `Alarm sound failed: ${s.sound_error}`);
  if (!s.settings.alarms.enabled) {
    el.replaceChildren(h("span", { class: "grow" }, h("strong", {}, "Alarms are off"), " · turn them on in Settings → Alarms"));
    return;
  }
  if (!q) {
    el.replaceChildren(...[h("span", { class: "grow muted" }, "Alarms: normal"), quietBtn, soundErr].filter(Boolean));
    return;
  }
  const off = [!q.allow.flash && "flashing", !q.allow.notify && "notifications", !q.allow.sound && "sound"].filter(Boolean);
  el.replaceChildren(
    ...[
      h(
        "span",
        { class: "grow" },
        h("strong", {}, q.name),
        q.manual_until ? ` until ${new Date(q.manual_until).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : "",
        off.length ? ` · ${off.join(", ")} off` : " · all alarm actions on",
      ),
      q.manual_until ? h("button", { onclick: () => invoke("set_quiet", { minutes: 0 }) }, "End quiet") : quietBtn,
      soundErr,
    ].filter(Boolean),
  );
}

onSnapshot((s) => {
  snap = s;
  document.getElementById("demo").hidden = !s.demo_mode;
  showQuiet(s);
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
