import { invoke, onSnapshot, h, ageText, displayValue, avatar } from "./common.js";

const TABS = [
  ["people", "People"],
  ["tray", "Tray icon"],
  ["badge", "Desktop badge"],
  ["general", "General"],
  ["about", "About"],
];
const COLORS = ["#4C6FFF", "#E0559B", "#14A3A3", "#9B59D0", "#E07A2E", "#2E9E5B"];
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
  return h("label", { class: "field" }, label, input, hint && h("span", { class: "small muted" }, hint));
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
            wizard = null;
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
  if (name === "add") {
    tab = "people";
    startWizard("add");
  } else if (TABS.some(([id]) => id === name)) {
    tab = name;
    wizard = null;
  }
  if (snap) render();
}

function render() {
  if (!snap) return;
  renderNav();
  const views = { people: wizard ? wizardView : peopleView, tray: trayView, badge: badgeView, general: generalView, about: aboutView };
  main.replaceChildren(views[tab]());
}

// ---------- People ----------

function statusText(p) {
  if (p.value) return [p.unit_label, p.arrow, "·", ageText(p.age_min)].filter(Boolean).join(" ");
  return p.message ?? "No recent reading";
}

function peopleView() {
  const rows = snap.people.map((p) => {
    const cfg = snap.config_people.find((c) => c.id === p.id);
    let confirming = false;
    const actions = h("div", { class: "actions" });
    const drawActions = () =>
      actions.replaceChildren(
        ...(confirming
          ? [
              h("span", { class: "small" }, `Remove ${p.name} and their saved sign-in?`),
              h("button", { class: "danger", onclick: () => invoke("remove_person", { id: p.id }) }, "Remove"),
              h("button", { onclick: () => ((confirming = false), drawActions()) }, "Cancel"),
            ]
          : [
              h("button", { onclick: () => startWizard("edit", cfg) }, "Edit"),
              !p.is_demo && h("button", { onclick: () => startWizard("signin", cfg) }, "Sign in again"),
              h("button", { class: "danger", onclick: () => ((confirming = true), drawActions()) }, "Remove"),
            ]
        ).filter(Boolean),
      );
    drawActions();
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
          h("div", {}, h("strong", {}, p.name), " ", p.is_demo && h("span", { class: "demo-pill" }, "DEMO")),
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
  const title = { add: "Add a person", signin: "Sign in again", edit: "Edit person" }[w.mode];
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
}

function stepSource() {
  const opt = (kind, title, text, tag) =>
    h(
      "div",
      { class: "option", role: "radio", tabindex: 0, "aria-checked": String(wizard.kind === kind), onclick: () => ((wizard.kind = kind), render()) },
      h("div", {}, h("strong", {}, title), tag && h("span", { class: "tag" }, tag), h("p", {}, text)),
    );
  return [
    h("p", { class: "muted" }, "Where do this person's readings come from?"),
    opt("nightscout", "Nightscout", "Uses a read-only access token. No Dexcom password needed, and you can revoke the token any time.", "Recommended"),
    opt("dexcom_share", "Dexcom Share", "Uses the Dexcom login of the person wearing the sensor, not a follower account. Share must be on with at least one follower."),
    opt("demo", "Demo data", "Synthetic readings for trying the app. No account or internet needed. Always marked DEMO."),
    h("p", { class: "small muted" }, "The official Dexcom API isn't offered: its readings arrive 1 to 3 hours late."),
    h("div", { class: "actions" }, h("button", { onclick: () => ((wizard = null), render()) }, "Cancel"), h("button", { class: "primary", onclick: () => go(2) }, "Next")),
  ];
}

// Secret inputs are kept out of `wizard` state and cleared right after the test.
let secretInputs = {};

function stepDetails() {
  const w = wizard;
  const back = h("button", { onclick: () => (w.mode === "signin" ? ((wizard = null), render()) : go(1)) }, "Back");
  const test = h("button", { class: "primary", disabled: w.busy, onclick: runTest }, w.busy ? "Testing…" : "Test connection");
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
    secretInputs = { token: h("input", { type: "password", id: "ns-token", autocomplete: "off", placeholder: "readable-abc123…" }) };
    return [
      field("Nightscout address", h("input", { type: "url", id: "ns-url", value: w.url, placeholder: "https://my-site.example", oninput: (e) => (w.url = e.target.value) })),
      field("Access token (read-only role)", secretInputs.token, "Leave empty if the site is public."),
      keychain,
      h("div", { class: "actions" }, back, test),
    ];
  }
  secretInputs = { password: h("input", { type: "password", id: "dx-password", autocomplete: "off" }) };
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
  } catch (e) {
    w.error = String(e);
  } finally {
    w.busy = false;
    for (const input of Object.values(secretInputs)) input.value = "";
    secretInputs = {};
    w.username = "";
    render();
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
      r.value ? h("div", { class: "big value st-in_range" }, `${r.value} ${r.arrow}`) : h("div", {}, "No reading"),
      r.value && h("div", { class: "small muted" }, `mg/dL · ${ageText(r.age_min)}`),
      r.note && h("p", { class: "small" }, r.note),
    ),
    h(
      "p",
      {},
      demo ? "This is synthetic data. It will always be marked DEMO." : h("strong", {}, "Does this match the Dexcom app right now? If not, go back and check the account and region."),
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
  const num = (key) => h("input", { type: "number", id: `t-${key}`, min: 40, max: 400, value: t[key], oninput: (e) => (t[key] = Number(e.target.value)) });
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
    field("Color", h("div", { class: "swatches" }, COLORS.map((c) => h("button", { class: "swatch", style: `background:${c}`, "aria-label": c, "aria-pressed": String(d.color === c), onclick: () => ((d.color = c), render()) })))),
    field("Units", seg([["mgdl", "mg/dL"], ["mmol", "mmol/L"]], d.units, (v) => ((d.units = v), render()))),
    field("Ranges (mg/dL)", h("div", { class: "row" }, "Urgent low below", num("urgent_low"), "Low below", num("low"), "High above", num("high"))),
    check("Own icon in the tray", d.tray_icon, (v) => (d.tray_icon = v), "tray-own"),
    h(
      "div",
      { class: "actions" },
      h("button", { onclick: () => (wizard.mode === "edit" ? ((wizard = null), render()) : go(3)) }, "Back"),
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
    field("With several people", seg([["per_person", "One icon per person"], ["primary", "One icon for everyone"]], t.mode, (v) => save((s) => (s.tray.mode = v)))),
    h("p", { class: "small muted" }, "With one icon per person, turn a person's icon off under People → Edit."),
    field(
      "Main person (shown on the single icon)",
      h(
        "select",
        { id: "primary", onchange: (e) => save((s) => (s.tray.primary = e.target.value || null)) },
        h("option", { value: "" }, "First in the list"),
        snap.people.map((p) => h("option", { value: p.id, selected: p.id === t.primary }, p.name)),
      ),
    ),
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
          s.badge.people = chosen.length === all.length ? [] : chosen;
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
    h("h3", {}, "People"),
    people,
    seg([[false, "One badge each"], [true, "Combined"]], b.combined, set("combined")),
    h("h3", {}, "Style"),
    seg([["dot", "Color only"], ["number", "Color + number"], ["arrow", "+ Trend arrow"], ["age", "+ Reading age"], ["card", "Card with trend"]], b.style, set("style")),
    h("h3", {}, "Size"),
    seg([["s", "Small"], ["m", "Medium"], ["l", "Large"]], b.size, set("size")),
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

openTab(location.hash.slice(1) || "people");
window.__TAURI__.event.listen("open-tab", (e) => openTab(e.payload));
onSnapshot((s) => {
  const first = !snap;
  snap = s;
  // Live data matters on the People list; other tabs redraw only when changed here.
  if (first || (tab === "people" && !wizard)) render();
});
