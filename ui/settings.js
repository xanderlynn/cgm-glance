import { invoke, onSnapshot, h, ageText, displayValue, avatar, statusChip } from "./common.js";

const TABS = [
  ["people", "People"],
  ["tray", "Tray icon"],
  ["badge", "Desktop badge"],
  ["alarms", "Alarms"],
  ["general", "General"],
  ["about", "About"],
];
const COLORS = ["#4C6FFF", "#E0559B", "#14A3A3", "#9B59D0", "#E07A2E", "#2E9E5B"];
const COLOR_NAMES = ["Blue", "Pink", "Teal", "Purple", "Orange", "Green"];
const SCENARIOS = [
  ["normal", "Realistic day (moves through all ranges)"],
  ["steady", "Steady, in range"],
  ["high", "High"],
  ["low", "Low"],
  ["urgent_low", "Urgent low"],
  ["stale", "Old reading (14 min)"],
  ["no_data", "No data (35 min)"],
  ["sign_in", "Rejected password"],
  ["offline", "No internet"],
];

let snap = null;
let tab = "people";
let wizard = null; // set while adding a person, signing in again, or editing
let confirmRemove = null; // id of the person whose removal is being confirmed
let shownKey = ""; // what the People list currently shows, to skip needless redraws
const main = document.getElementById("main");
const nav = document.getElementById("nav");

// ---------- small building blocks ----------

function seg(options, current, onChange) {
  return h(
    "div",
    { class: "seg", role: "group" },
    options.map(([value, label]) =>
      h("button", { "aria-pressed": String(value === current), onclick: () => onChange(value) }, label),
    ),
  );
}

function check(label, checked, onChange, id) {
  return h(
    "label",
    { class: "check" },
    h("input", { type: "checkbox", id, checked, onchange: (e) => onChange(e.target.checked) }),
    label,
  );
}

function field(label, input, hint) {
  const hintEl = hint && h("span", { class: "small muted" }, hint);
  // A <label> around a group of buttons would make clicking the label text press the first button.
  if (input instanceof HTMLInputElement || input instanceof HTMLSelectElement) return h("label", { class: "field" }, label, input, hintEl);
  return h("div", { class: "field", role: "group", "aria-label": label }, h("span", {}, label), input, hintEl);
}

/** Leaves the wizard and forgets any tested sign-in. */
function closeWizard() {
  wizard = null;
  for (const input of Object.values(secretInputs)) input.value = "";
  secretInputs = {};
  invoke("discard_drafts");
}

/** Changes settings, saves them, and redraws the current tab. */
function save(mutate) {
  const settings = structuredClone(snap.settings);
  mutate(settings);
  snap.settings = settings;
  invoke("save_settings", { settings }).catch((e) => alertBox(String(e)));
  render();
}

function alertBox(msg) {
  main.prepend(h("p", { class: "error" }, msg));
}

// ---------- navigation ----------

function renderNav() {
  nav.replaceChildren(
    ...TABS.map(([id, label]) =>
      h(
        "button",
        {
          "aria-current": tab === id ? "page" : null,
          onclick: () => {
            tab = id;
            if (wizard) closeWizard();
            render();
          },
        },
        label,
      ),
    ),
    snap?.demo_mode && h("span", { class: "demo-pill" }, "DEMO MODE"),
  );
}

function openTab(name) {
  const signinId = name.startsWith("signin:") ? name.slice(7) : null;
  const person = signinId && snap?.config_people.find((p) => p.id === signinId);
  if (name === "add") {
    tab = "people";
    startWizard("add");
  } else if (person) {
    startWizard("signin", person);
  } else if (TABS.some(([id]) => id === name)) {
    tab = name;
    if (wizard) closeWizard();
  }
  if (snap) render();
}

function render() {
  if (!snap) return;
  shownKey = listKey(snap);
  const focusKey = document.activeElement?.dataset?.key;
  renderNav();
  const views = { people: wizard ? wizardView : peopleView, tray: trayView, badge: badgeView, alarms: alarmsView, general: generalView, about: aboutView };
  main.replaceChildren(views[tab]());
  if (focusKey) main.querySelector(`[data-key="${CSS.escape(focusKey)}"]`)?.focus();
}

// ---------- People ----------

function statusText(p) {
  if (p.value) return [p.unit_label, p.arrow, "·", ageText(p.age_min)].filter(Boolean).join(" ");
  return p.message ?? "No recent reading";
}

function peopleView() {
  const rows = snap.people.map((p) => {
    const cfg = snap.config_people.find((c) => c.id === p.id);
    const actions = h(
      "div",
      { class: "actions" },
      ...(confirmRemove === p.id
        ? [
            h("span", { class: "small" }, `Remove ${p.name} and their saved sign-in?`),
            h("button", { class: "danger", onclick: () => ((confirmRemove = null), invoke("remove_person", { id: p.id })) }, "Yes, remove"),
            h("button", { "data-key": `${p.id}:cancel`, onclick: () => ((confirmRemove = null), render()) }, "Cancel"),
          ]
        : [
            h("button", { "data-key": `${p.id}:edit`, onclick: () => startWizard("edit", cfg) }, "Edit"),
            !p.is_demo && h("button", { "data-key": `${p.id}:signin`, onclick: () => startWizard("signin", cfg) }, "Sign in again"),
            h("button", { "data-key": `${p.id}:remove`, class: "danger", onclick: () => ((confirmRemove = p.id), render()) }, "Remove"),
          ]
      ).filter(Boolean),
    );
    return h(
      "div",
      { class: `person st-${p.status}` },
      h(
        "div",
        { class: "row" },
        avatar(p),
        h(
          "div",
          { class: "grow" },
          h("div", {}, h("strong", {}, p.name), " ", p.is_demo && h("span", { class: "demo-pill" }, "DEMO"), " ", statusChip(p)),
          h("div", { class: "small muted" }, p.source_label),
        ),
        h("div", { class: "small" }, h("span", { class: "value" }, displayValue(p)), " ", statusText(p)),
      ),
      h("div", { style: "margin-top:8px" }, actions),
    );
  });
  return h(
    "section",
    {},
    h("h2", {}, "People"),
    rows.length ? rows : h("div", { class: "note" }, "No one added yet. Add the person wearing the sensor, or a demo person to try the app."),
    h("div", {}, h("button", { class: "primary", onclick: () => startWizard("add") }, "+ Add person")),
  );
}

// ---------- Add / sign in again / edit wizard ----------

function startWizard(mode, person) {
  secretInputs = {};
  const kind = person?.source.kind ?? "nightscout";
  wizard = {
    mode,
    step: mode === "edit" ? 4 : mode === "signin" ? 2 : 1,
    kind,
    region: person?.source.region ?? "us",
    scenario: person?.source.scenario ?? "normal",
    url: person?.source.url ?? "",
    username: "",
    result: null,
    error: "",
    busy: false,
    personId: person?.id,
    autoInitials: mode === "add",
    display: person
      ? { name: person.name, initials: person.initials, color: person.color, units: person.units, thresholds: { ...person.thresholds }, tray_icon: person.tray_icon }
      : { name: "", initials: "", color: COLORS[snap.people.length % COLORS.length], units: "mgdl", thresholds: { urgent_low: 55, low: 70, high: 180 }, tray_icon: true },
  };
  tab = "people";
  render();
}

function wizardView() {
  const w = wizard;
  const title = { add: "Add a person", signin: `Sign in again: ${w.display.name}`, edit: `Edit ${w.display.name}` }[w.mode];
  const body = [null, stepSource, stepDetails, stepResult, stepDisplay][w.step]();
  return h(
    "section",
    {},
    h("h2", {}, title),
    w.mode === "add" && h("div", { class: "steps" }, `Step ${w.step} of 4`),
    body,
    w.error && h("p", { class: "error", role: "alert" }, w.error),
  );
}

function go(step) {
  wizard.step = step;
  wizard.error = "";
  render();
  // Keyboard and screen-reader users land on the new step, not at the top of the page.
  main.querySelector("input:not([type=checkbox]), select, [role=radio][aria-checked=true]")?.focus();
}

// Enter in a text field runs the step's main action.
main.addEventListener("keydown", (e) => {
  if (e.key !== "Enter" || e.target.tagName !== "INPUT" || !wizard || wizard.busy) return;
  if (wizard.step === 2) runTest();
  else if (wizard.step === 4) saveWizard();
});

function stepSource() {
  const opt = (kind, title, text, tag) =>
    h(
      "div",
      {
        class: "option",
        role: "radio",
        tabindex: 0,
        "aria-label": title,
        "aria-checked": String(wizard.kind === kind),
        onclick: () => ((wizard.kind = kind), render()),
        onkeydown: (e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            wizard.kind = kind;
            render();
            main.querySelector(`[data-kind="${kind}"]`)?.focus();
          }
        },
        "data-kind": kind,
      },
      h("div", {}, h("strong", {}, title), tag && h("span", { class: "tag" }, tag), h("p", {}, text)),
    );
  return [
    h("p", { class: "muted", id: "source-q" }, "Where do this person's readings come from?"),
    h("div", { role: "radiogroup", "aria-labelledby": "source-q", style: "display:flex;flex-direction:column;gap:10px" }, opt("nightscout", "Nightscout", "Uses a read-only access token. No Dexcom password needed, and you can revoke the token any time.", "Recommended"),
    opt("dexcom_share", "Dexcom Share", "Uses the Dexcom login of the person wearing the sensor, not a follower account. Share must be on with at least one follower."),
    opt("demo", "Demo data", "Synthetic readings for trying the app. No account or internet needed. Always marked DEMO.")),
    h("p", { class: "small muted" }, "The official Dexcom API isn't offered: its readings arrive 1 to 3 hours late."),
    h("div", { class: "actions" }, h("button", { onclick: () => (closeWizard(), render()) }, "Cancel"), h("button", { class: "primary", onclick: () => go(2) }, "Next")),
  ];
}

// Secret inputs are kept out of `wizard` state and cleared right after the test.
let secretInputs = {};

function stepDetails() {
  const w = wizard;
  const back = h("button", { onclick: () => (w.mode === "signin" ? (closeWizard(), render()) : go(1)) }, "Back");
  const testLabel = w.kind === "demo" ? "Load demo data" : "Test connection";
  const test = h("button", { class: "primary", disabled: w.busy, onclick: runTest }, w.busy ? "Testing…" : testLabel);
  const keychain = h(
    "div",
    { class: "note safe" },
    h("strong", {}, "Stored in your system keychain only. "),
    "Windows Credential Manager, macOS Keychain or Linux Secret Service. Never written to a file or log, and sent only to the service you chose.",
  );
  if (w.kind === "demo") {
    return [
      field(
        "Scenario",
        h("select", { id: "scenario", onchange: (e) => (w.scenario = e.target.value) }, SCENARIOS.map(([v, l]) => h("option", { value: v, selected: v === w.scenario }, l))),
        "Every scenario is generated by formula. Use them to check each state of the tray and badge.",
      ),
      h("div", { class: "actions" }, back, test),
    ];
  }
  if (w.kind === "nightscout") {
    secretInputs.token ??= h("input", { type: "password", id: "ns-token", autocomplete: "off", placeholder: "readable-abc123…" });
    return [
      field("Nightscout address", h("input", { type: "url", id: "ns-url", value: w.url, placeholder: "https://my-site.example", oninput: (e) => (w.url = e.target.value) })),
      field("Access token (read-only role)", secretInputs.token, "Leave empty if the site is public."),
      keychain,
      h("div", { class: "actions" }, back, test),
    ];
  }
  secretInputs.password ??= h("input", { type: "password", id: "dx-password", autocomplete: "off" });
  return [
    field("Region", seg([["us", "US"], ["ous", "Outside US"], ["jp", "Japan"]], w.region, (v) => ((w.region = v), render()))),
    field("Email, username or phone", h("input", { type: "text", id: "dx-user", autocomplete: "off", value: w.username, oninput: (e) => (w.username = e.target.value) }), "Phone numbers need + and the country code."),
    field("Password", secretInputs.password),
    keychain,
    h("div", { class: "note warn small" }, "If the password is wrong, the app stops after one try so it can't lock the Dexcom account."),
    h("div", { class: "actions" }, back, test),
  ];
}

async function runTest() {
  const w = wizard;
  const source =
    w.kind === "demo"
      ? { kind: "demo", scenario: w.scenario }
      : w.kind === "nightscout"
        ? { kind: "nightscout", url: w.url, token: secretInputs.token?.value || null }
        : { kind: "dexcom_share", region: w.region, username: w.username, password: secretInputs.password?.value ?? "" };
  w.busy = true;
  w.error = "";
  render();
  try {
    w.result = await invoke("test_connection", { source });
    w.step = 3;
    w.username = ""; // not needed after a successful test
  } catch (e) {
    w.error = String(e);
  } finally {
    w.busy = false;
    for (const input of Object.values(secretInputs)) input.value = "";
    secretInputs = {};
    render();
    // After a test, focus the next action (or the field to fix).
    main.querySelector(w.step === 3 ? ".actions .primary" : "input, select")?.focus();
  }
}

function stepResult() {
  const r = wizard.result;
  const demo = wizard.kind === "demo";
  return [
    h("div", { class: "row" }, h("strong", {}, `Connected to ${r.source_label}`)),
    h(
      "div",
      { class: "note" },
      h("div", { class: "small muted" }, "Latest reading"),
      r.value ? h("div", { class: `big value st-${r.status}` }, `${r.value} ${r.arrow}`) : h("div", {}, "No reading"),
      r.value && h("div", { class: "small muted" }, `mg/dL · ${ageText(r.age_min)}`),
      r.note && h("p", { class: "small" }, r.note),
    ),
    h(
      "p",
      {},
      demo
        ? "This is synthetic data. It will always be marked DEMO."
        : h(
            "strong",
            {},
            wizard.kind === "nightscout"
              ? "Does this match the Dexcom app right now? If not, go back and check the address."
              : "Does this match the Dexcom app right now? If not, go back and check the account and region.",
          ),
    ),
    h(
      "div",
      { class: "actions" },
      h("button", { onclick: () => go(2) }, "Back"),
      wizard.mode === "signin"
        ? h("button", { class: "primary", onclick: () => saveWizard() }, "Yes, save")
        : h("button", { class: "primary", onclick: () => go(4) }, "Yes, continue"),
    ),
  ];
}

function stepDisplay() {
  const d = wizard.display;
  const t = d.thresholds;
  const initialsInput = h("input", {
    type: "text",
    id: "initials",
    maxlength: 3,
    value: d.initials,
    style: "width:80px",
    oninput: (e) => {
      d.initials = e.target.value;
      wizard.autoInitials = false;
    },
  });
  // Ranges are stored in mg/dL but shown and typed in the person's units.
  const mmol = d.units === "mmol";
  const MMOL = 18.0182;
  const num = (key) =>
    h("input", {
      type: "number",
      id: `t-${key}`,
      "aria-label": { urgent_low: "Urgent low below", low: "Low below", high: "High above" }[key],
      step: mmol ? 0.1 : 1,
      min: mmol ? 2.2 : 40,
      max: mmol ? 22.2 : 400,
      value: mmol ? (t[key] / MMOL).toFixed(1) : t[key],
      oninput: (e) => (t[key] = Math.round(mmol ? Number(e.target.value) * MMOL : Number(e.target.value))),
    });
  return [
    h(
      "div",
      { class: "row" },
      field(
        "Name",
        h("input", {
          type: "text",
          id: "name",
          maxlength: 40,
          value: d.name,
          oninput: (e) => {
            d.name = e.target.value;
            if (wizard.autoInitials) initialsInput.value = d.initials = d.name.trim().slice(0, 2).toUpperCase();
          },
        }),
      ),
      field("Initials", initialsInput),
    ),
    field("Color", h("div", { class: "swatches" }, COLORS.map((c, i) => h("button", { class: "swatch", style: `background:${c}`, "aria-label": COLOR_NAMES[i], title: COLOR_NAMES[i], "aria-pressed": String(d.color === c), onclick: () => ((d.color = c), render()) })))),
    field("Units", seg([["mgdl", "mg/dL"], ["mmol", "mmol/L"]], d.units, (v) => ((d.units = v), render()))),
    field(`Ranges (${mmol ? "mmol/L" : "mg/dL"})`, h("div", { class: "row" }, "Urgent low below", num("urgent_low"), "Low below", num("low"), "High above", num("high"))),
    check("Own icon in the tray", d.tray_icon, (v) => (d.tray_icon = v), "tray-own"),
    h(
      "div",
      { class: "actions" },
      h("button", { onclick: () => (wizard.mode === "edit" ? (closeWizard(), render()) : go(3)) }, "Back"),
      h("button", { class: "primary", onclick: saveWizard }, "Save"),
    ),
  ];
}

async function saveWizard() {
  const w = wizard;
  w.error = "";
  try {
    if (w.mode === "edit") {
      await invoke("update_person", { id: w.personId, display: w.display });
    } else {
      await invoke("save_person", { draftId: w.result.draft_id, display: w.display, replaceId: w.mode === "signin" ? w.personId : null });
    }
    wizard = null;
    render();
  } catch (e) {
    w.error = String(e);
    render();
  }
}

// ---------- Tray ----------

function trayView() {
  const t = snap.settings.tray;
  return h(
    "section",
    {},
    h("h2", {}, "Tray icon"),
    field(
      "With several people",
      seg([["per_person", "One icon per person"], ["primary", "One icon, most urgent"], ["single", "One icon, one person"]], t.mode, (v) => save((s) => (s.tray.mode = v))),
    ),
    h(
      "p",
      { class: "small muted" },
      {
        per_person: "Each person gets their own icon. Turn a person's icon off under People → Edit.",
        primary: "One icon shows whoever needs attention most right now. Click it to see everyone.",
        single: "One icon shows only the person you choose. Click it to see everyone.",
      }[t.mode],
    ),
    t.mode === "single" &&
    field(
      "Person shown on the icon",
      h(
        "select",
        { id: "primary", onchange: (e) => save((s) => (s.tray.primary = e.target.value || null)) },
        h("option", { value: "" }, "First in the list"),
        snap.people.map((p) => h("option", { value: p.id, selected: p.id === t.primary }, p.name)),
      ),
    ),
    check("Show initials beside the number", t.show_initials, (v) => save((s) => (s.tray.show_initials = v)), "tray-initials"),
    field("Icon on Windows", seg([["number", "Color + number"], ["dot", "Color dot only"]], t.icon_style, (v) => save((s) => (s.tray.icon_style = v)))),
    h("p", { class: "small muted" }, "macOS and Linux always show a color dot with the number as text beside it. Hover (Windows) or click for details."),
  );
}

// ---------- Desktop badge ----------

function badgeView() {
  const b = snap.settings.badge;
  const set = (k) => (v) => save((s) => (s.badge[k] = v));
  const people = h(
    "div",
    { class: "actions" },
    snap.people.map((p) =>
      check(p.name, b.people.length === 0 || b.people.includes(p.id), (on) =>
        save((s) => {
          const all = snap.people.map((q) => q.id);
          let chosen = s.badge.people.length ? s.badge.people : all;
          chosen = on ? [...new Set([...chosen, p.id])] : chosen.filter((id) => id !== p.id);
          // Empty means "everyone", so unchecking the last person turns the badge off instead.
          if (chosen.length === 0) s.badge.enabled = false;
          s.badge.people = chosen.length === all.length || chosen.length === 0 ? [] : chosen;
        }),
        `badge-p-${p.id}`,
      ),
    ),
  );
  return h(
    "section",
    {},
    h("h2", {}, "Desktop badge"),
    check(h("strong", {}, "Show desktop badge"), b.enabled, set("enabled"), "badge-on"),
    h(
      "fieldset",
      { disabled: !b.enabled, class: "group" },
    h("h3", {}, "People"),
    people,
    seg([[false, "One badge each"], [true, "Combined"]], b.combined, set("combined")),
    h("h3", {}, "Style"),
    seg([["pill", "Pill"], ["ring", "Ring"], ["faces", "Faces (no number)"]], b.style, set("style")),
    b.style === "faces" &&
      h("p", { class: "small muted" }, "Shows a face instead of the number: 🙂 in range, 😬 high, 😟 low, 😱 urgent low, 😴 old reading. Hover the badge or open the tray to see the number."),
    h("h3", {}, "Size"),
    seg([["s", "Small"], ["m", "Medium"], ["l", "Large"]], b.size, set("size")),
    check("Show initials", b.show_initials, set("show_initials"), "badge-initials"),
    h("h3", {}, "Layer"),
    seg([["top", "Always on top"], ["normal", "Normal"], ["bottom", "Behind windows"]], b.layer, set("layer")),
    h("h3", {}, "Position"),
    seg([["free", "Free (drag it)"], ["edge", "Screen edge"], ["corner", "Corner"]], b.position, set("position")),
    b.position === "edge" && seg([["left", "Left"], ["right", "Right"], ["top", "Top"], ["bottom", "Bottom"]], b.edge, set("edge")),
    b.position === "corner" && seg([["top_left", "Top left"], ["top_right", "Top right"], ["bottom_left", "Bottom left"], ["bottom_right", "Bottom right"]], b.corner, set("corner")),
    b.position === "free" && h("p", { class: "small muted" }, "Drag the badge anywhere. It snaps to a screen edge when you drop it close to one."),
    h("h3", {}, "Behavior"),
    check("Lock position", b.lock, set("lock"), "badge-lock"),
    check("Clicks pass through the badge", b.click_through, set("click_through"), "badge-ct"),
    b.click_through && h("p", { class: "small muted" }, "The badge can't be dragged while clicks pass through. Turn this off here to move it."),
    field(
      `Opacity ${Math.round(b.opacity * 100)}%`,
      h("input", { type: "range", id: "badge-opacity", min: 30, max: 100, value: Math.round(b.opacity * 100), onchange: (e) => set("opacity")(Number(e.target.value) / 100) }),
    ),
    h("div", {}, h("button", { onclick: () => invoke("reset_badge_positions") }, "Reset badge positions")),
    h("p", { class: "small muted" }, "Linux on Wayland decides window position and stacking itself, so position and layer may not apply there."),
    ),
  );
}

// ---------- Alarms ----------

const MMOL = 18.0182;

function alarmsView() {
  const a = snap.settings.alarms;
  // Rules are shared, so numbers use mmol/L only when everyone does.
  const mmol = snap.config_people.length > 0 && snap.config_people.every((p) => p.units === "mmol");
  const unit = mmol ? "mmol/L" : "mg/dL";
  const shown = (mg) => (mmol ? (mg / MMOL).toFixed(1) : String(mg));
  const set = (fn) => save((s) => fn(s.alarms));
  const phaseRow = (kind, ph, i) => {
    const id = `${kind}-${i}`;
    const edit = (fn) => set((al) => fn(al[kind][i]));
    return h(
      "div",
      { class: `phase${ph.enabled ? "" : " off"}` },
      h("input", { type: "checkbox", checked: ph.enabled, "aria-label": "Phase on", "data-key": `${id}-on`, onchange: (e) => edit((p) => (p.enabled = e.target.checked)) }),
      h("span", {}, kind === "low" ? "At or below" : "At or above"),
      h("input", {
        type: "number",
        step: mmol ? "0.1" : "1",
        min: mmol ? 2.2 : 40,
        max: mmol ? 22.2 : 400,
        value: shown(ph.mgdl),
        "aria-label": `Threshold in ${unit}`,
        "data-key": `${id}-n`,
        onchange: (e) => {
          const n = Number(e.target.value);
          if (!Number.isFinite(n) || n <= 0) return render();
          edit((p) => (p.mgdl = Math.round(mmol ? n * MMOL : n)));
        },
      }),
      h("span", { class: "muted" }, unit),
      check(kind === "low" ? "only when falling" : "only when rising", ph.trend_only, (v) => edit((p) => (p.trend_only = v)), `${id}-trend`),
      h("span", { class: "grow" }),
      h("span", { class: "muted small" }, "Then:"),
      check("Flash", ph.actions.flash, (v) => edit((p) => (p.actions.flash = v)), `${id}-flash`),
      check("Notify", ph.actions.notify, (v) => edit((p) => (p.actions.notify = v)), `${id}-notify`),
      h("button", { "aria-label": "Remove this phase", title: "Remove this phase", onclick: () => set((al) => al[kind].splice(i, 1)) }, "✕"),
    );
  };
  const phases = (kind, title) => [
    h("h3", {}, title),
    a[kind].length === 0 && h("p", { class: "small" }, "No phases: this alarm is off."),
    ...a[kind].map((ph, i) => phaseRow(kind, ph, i)),
    a[kind].length < 6 &&
      h(
        "div",
        {},
        h("button", {
          onclick: () =>
            set((al) => {
              const last = al[kind].at(-1)?.mgdl;
              const mgdl = kind === "low" ? Math.max(40, (last ?? 80) - 5) : Math.min(400, (last ?? 180) + 20);
              al[kind].push({ enabled: true, mgdl, trend_only: false, actions: { flash: true, notify: true } });
            }),
        }, "+ Add phase"),
      ),
  ];
  return h(
    "section",
    {},
    h("h2", {}, "Alarms"),
    h(
      "div",
      { class: "note warn" },
      "These alarms add to the Dexcom app's own alarms; they don't replace them. Dexcom Share data can arrive minutes late, and a computer can sleep or lose its connection.",
    ),
    check(h("strong", {}, "Alarms on"), a.enabled, (v) => set((al) => (al.enabled = v)), "alarms-on"),
    h(
      "fieldset",
      { disabled: !a.enabled, class: "group" },
      h("h3", {}, "People"),
      snap.people.length === 0
        ? h("p", { class: "small muted" }, "Add a person first.")
        : h(
            "div",
            { class: "actions" },
            snap.people.map((p) =>
              check(p.name, !a.off_for.includes(p.id), (on) =>
                set((al) => (al.off_for = on ? al.off_for.filter((x) => x !== p.id) : [...al.off_for, p.id])),
                `alarm-p-${p.id}`,
              ),
            ),
          ),
      h("p", { class: "small muted" }, "Phases escalate: the most extreme phase a reading is in decides what happens, and each new, more extreme phase alerts again. An alarm ends once the reading is 5 mg/dL back past its first phase. Readings older than 10 minutes never trigger an alarm."),
      ...phases("low", "Low alarm"),
      ...phases("high", "High alarm"),
      h("h3", {}, "Repeat"),
      field(
        "Alert again every (minutes) until acknowledged",
        h("input", {
          type: "number", id: "alarm-repeat", min: 1, max: 60, value: a.repeat_min,
          onchange: (e) => { const n = Math.round(Number(e.target.value)); if (n >= 1 && n <= 60) set((al) => (al.repeat_min = n)); else render(); },
        }),
      ),
      h("p", { class: "small muted" }, "Acknowledge an alarm from the readings popup or the tray menu. Notifications use your system's notification settings; check that CGM Glance is allowed to notify."),
    ),
  );
}

// ---------- General / About ----------

function generalView() {
  const g = snap.settings;
  return h(
    "section",
    {},
    h("h2", {}, "General"),
    check("Start CGM Glance when I sign in", g.autostart, (v) => save((s) => (s.autostart = v)), "autostart"),
    h("h3", {}, "Test mode"),
    snap.people.some((p) => p.is_demo) &&
      check("Fast demo data: a new reading every 10 seconds", g.demo_fast, (v) => save((s) => (s.demo_fast = v)), "demo-fast"),
    h(
      "div",
      { class: "note small" },
      "To try everything without an account, add a person with the Demo data source, or start the app with ",
      h("code", {}, "--demo"),
      ". Demo mode uses its own settings file, adds three demo people, and never touches your real settings or keychain.",
    ),
  );
}

function aboutView() {
  const version = h("span", {}, "…");
  window.__TAURI__.app.getVersion().then((v) => (version.textContent = v));
  return h(
    "section",
    {},
    h("h2", {}, "About"),
    h("p", {}, "CGM Glance ", version),
    h("div", { class: "note warn" }, h("strong", {}, "Not a medical device. "), "It shows readings at a glance and doesn't replace the Dexcom app or its alarms. Don't make treatment decisions from it alone."),
    h("h3", {}, "Privacy"),
    h(
      "ul",
      {},
      h("li", {}, "Passwords and tokens are kept only in the system keychain."),
      h("li", {}, "Settings (names, colors, ranges) are kept in a local file without any sign-in details."),
      h("li", {}, "The app talks only to Dexcom or your Nightscout site. No analytics or tracking."),
    ),
  );
}

// ---------- start ----------

window.__TAURI__.event.listen("open-tab", (e) => openTab(e.payload));
// Only what the People list shows. Redrawing on every update would replace buttons mid-click.
const listKey = (s) =>
  JSON.stringify([confirmRemove, s.people.map((p) => [p.id, p.name, p.initials, p.color, p.status, p.value, p.arrow, p.age_min, p.message, p.source_label])]);

onSnapshot((s) => {
  const first = !snap;
  snap = s;
  // The tab in the URL (e.g. #add from the popup) needs data, so open it on the first update.
  if (first) openTab(location.hash.slice(1) || "people");
  // Live data matters on the People list; other tabs redraw only when changed here.
  else if (tab === "people" && !wizard && listKey(s) !== shownKey) render();
});
