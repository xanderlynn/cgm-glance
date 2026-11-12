// Fake window.__TAURI__ for testing the UI pages in a browser.
// Mirrors the Rust commands closely enough to exercise every screen. Synthetic data only.
// State is shared between the harness iframes through localStorage.
(() => {
  const KEY = "cgm-harness-state";
  const params = new URLSearchParams(location.search);
  const page = location.pathname.split("/").pop().replace(".html", "");
  const label = params.get("label") || (page === "badge" ? "badge-all" : page);
  const MIN = 60_000;
  const log = (window.__harnessLog = []);

  const defaults = () => ({
    tray: { mode: "per_person", primary: null, icon_style: "number" },
    badge: { enabled: true, people: [], combined: false, style: "pill", size: "m", layer: "top", position: "free", edge: "right", corner: "top_right", lock: false, click_through: false, opacity: 1 },
    demo_fast: true,
    autostart: false,
  });
  const person = (id, name, initials, color, scenario) => ({
    id, name, initials, color, units: "mgdl", thresholds: { urgent_low: 55, low: 70, high: 180 },
    source: { kind: "demo", scenario }, tray_icon: true,
  });
  const seed = () => ({
    start: Date.now(),
    people: [
      person("demo-a", "Alex (demo)", "AL", "#4C6FFF", "normal"),
      person("demo-m", "Mia (demo)", "MJ", "#E0559B", "low"),
      person("demo-s", "Sam (demo)", "SK", "#14A3A3", "stale"),
    ],
    settings: defaults(),
  });
  const load = () => {
    try { return JSON.parse(localStorage.getItem(KEY)) || seed(); } catch { return seed(); }
  };
  let state = load();
  if (!localStorage.getItem(KEY)) localStorage.setItem(KEY, JSON.stringify(state));
  const persist = () => { localStorage.setItem(KEY, JSON.stringify(state)); emitSnapshot(); };

  // ----- synthetic readings (same shapes as sources/demo.rs) -----
  const wave = (m, period, phase) => Math.sin(((m + phase) / period) * Math.PI * 2);
  function value(sc, m, s) {
    const v = { normal: 130 + 72 * wave(m, 360, s * 13) + 14 * wave(m, 47, s), stale: 130 + 72 * wave(m, 360, s * 13), no_data: 130,
      steady: 112 + 5 * wave(m, 60, s), high: 238 + 28 * wave(m, 120, s), low: 64 + 4 * wave(m, 40, s), urgent_low: 49 + 3 * wave(m, 30, s) }[sc] ?? 120;
    return Math.round(Math.min(401, Math.max(39, v)));
  }
  const ARROWS = (rate) => (rate > 3 ? ["⇈", 2] : rate > 2 ? ["↑", 1] : rate > 1 ? ["↗", 1] : rate >= -1 ? ["→", 0] : rate >= -2 ? ["↘", -1] : rate >= -3 ? ["↓", -1] : ["⇊", -2]);
  const toUnits = (mg, u) => (u === "mmol" ? Math.round((mg / 18.0182) * 10) / 10 : mg);
  const fmt = (mg, u) => (mg < 40 ? "LOW" : mg > 400 ? "HIGH" : u === "mmol" ? (mg / 18.0182).toFixed(1) : String(mg));

  function view(p, now) {
    const sc = p.source.kind === "demo" ? p.source.scenario : "steady";
    const base = { id: p.id, name: p.name, initials: p.initials, color: p.color, is_demo: p.source.kind === "demo",
      source_label: { demo: "Demo data", nightscout: "Nightscout", dexcom_share: "Dexcom Share (US)" }[p.source.kind],
      unit_label: p.units === "mmol" ? "mmol/L" : "mg/dL", low: toUnits(p.thresholds.low, p.units), high: toUnits(p.thresholds.high, p.units) };
    if (sc === "sign_in") return { ...base, status: "sign_in", value: null, icon_text: null, arrow: "", direction: 0, delta: null, age_min: null, message: "Demo: simulated wrong password. Use \"Sign in again\" to test that flow.", history: [] };
    if (sc === "offline") return { ...base, status: "error", value: null, icon_text: null, arrow: "", direction: 0, delta: null, age_min: null, message: "Demo: simulated network outage.", history: [] };
    const seedN = [...p.id].reduce((a, c) => a + c.charCodeAt(0), 0) % 997;
    const steps = Math.floor((now - state.start) / 10_000);
    const simNow = seedN * 37 + steps * 5;
    const lag = sc === "stale" ? 14 * MIN : sc === "no_data" ? 35 * MIN : 0;
    const newest = now - MIN - lag;
    const pts = [];
    for (let k = 35; k >= 0; k--) pts.push({ t: newest - k * 5 * MIN, mg: value(sc, simNow - 5 * k, seedN) });
    const last = pts[pts.length - 1], prev = pts[pts.length - 2];
    const age = Math.floor((now - last.t) / MIN);
    const t = p.thresholds;
    const status = age >= 20 ? "no_data" : age >= 10 ? "stale" : last.mg < t.urgent_low ? "urgent_low" : last.mg < t.low ? "low" : last.mg > t.high ? "high" : "in_range";
    const show = status !== "no_data";
    const fresh = !["stale", "no_data"].includes(status);
    const [arrow, direction] = fresh ? ARROWS((last.mg - prev.mg) / 5) : ["", 0];
    const d = last.mg - prev.mg;
    return { ...base, status, value: show ? fmt(last.mg, p.units) : null, icon_text: show ? fmt(last.mg, p.units) : null, arrow, direction,
      delta: fresh ? (p.units === "mmol" ? (d >= 0 ? "+" : "") + (d / 18.0182).toFixed(1) : (d >= 0 ? "+" : "") + d) : null,
      age_min: age, message: status === "no_data" ? "No recent reading. Check the sensor and the phone it sends to." : null,
      history: pts.map((q) => ({ t: q.t, v: toUnits(q.mg, p.units) })) };
  }
  const snapshot = () => {
    const now = Date.now();
    const settings = structuredClone(state.settings);
    // Gallery overrides: badge.html?style=card&size=l shows one style regardless of saved settings.
    for (const k of ["style", "size"]) if (params.get(k)) settings.badge[k] = params.get(k);
    if (params.get("style")) settings.badge.people = [];
    return { demo_mode: true, people: state.people.map((p) => view(p, now)), settings, config_people: structuredClone(state.people) };
  };

  // ----- events -----
  const listeners = {};
  function emit(name, payload) { (listeners[name] || []).forEach((cb) => cb({ payload })); }
  function emitSnapshot() { emit("changed", null); }
  window.addEventListener("storage", (e) => { if (e.key === KEY) { state = load(); emitSnapshot(); } });
  setInterval(emitSnapshot, 3000);

  // ----- commands -----
  const drafts = {};
  function validate(d) {
    if (!d.name.trim() || d.name.trim().length > 40) throw "Enter a name up to 40 characters.";
    const n = [...d.initials.trim()].length;
    if (n === 0 || n > 3) throw "Initials must be 1 to 3 characters.";
    if (!/^#[0-9a-fA-F]{6}$/.test(d.color)) throw "Pick a color.";
    const t = d.thresholds;
    if (!(40 <= t.urgent_low && t.urgent_low < t.low && t.low < t.high && t.high <= 400)) throw "Ranges must go urgent low < low < high, between 40 and 400 mg/dL (2.2 and 22.2 mmol/L).";
  }
  const apply = (p, d) => Object.assign(p, { name: d.name.trim(), initials: d.initials.trim().toUpperCase(), color: d.color, units: d.units, thresholds: { ...d.thresholds }, tray_icon: d.tray_icon });
  const commands = {
    get_snapshot: () => snapshot(),
    test_connection: async ({ source }) => {
      await new Promise((r) => setTimeout(r, 400));
      let cfg;
      if (source.kind === "dexcom_share") {
        if (!source.username.trim() || !source.password) throw "Enter the Dexcom username and password.";
        if (source.password === "wrong") throw "Dexcom did not accept this username and password, or the region is wrong.";
        cfg = { kind: "dexcom_share", region: source.region };
      } else if (source.kind === "nightscout") {
        const url = source.url.trim().replace(/\/+$/, "");
        if (!url.startsWith("https://") || url.length <= 8) throw "Enter the full Nightscout address starting with https://";
        if (source.token === "bad") throw "Nightscout refused the access token. Check it has the readable role.";
        cfg = { kind: "nightscout", url };
      } else cfg = { kind: "demo", scenario: source.scenario };
      const id = crypto.randomUUID();
      drafts[id] = cfg;
      const note = cfg.scenario === "sign_in" ? "Demo: simulated wrong password. This demo person will show that state." : null;
      return { draft_id: id, status: note ? "sign_in" : "in_range", source_label: cfg.kind, value: note ? null : "142", mgdl: note ? null : 142, arrow: note ? "" : "→", age_min: note ? null : 2, note };
    },
    save_person: ({ draftId, display, replaceId }) => {
      validate(display);
      const src = drafts[draftId];
      if (!src) throw "That connection test has expired. Test the connection again.";
      delete drafts[draftId];
      let p = state.people.find((q) => q.id === replaceId);
      if (!p) { p = { id: crypto.randomUUID(), source: src, tray_icon: true }; state.people.push(p); }
      p.source = src;
      apply(p, display);
      persist();
      return p.id;
    },
    update_person: ({ id, display }) => { validate(display); apply(state.people.find((p) => p.id === id), display); persist(); },
    remove_person: ({ id }) => { state.people = state.people.filter((p) => p.id !== id); state.settings.badge.people = state.settings.badge.people.filter((x) => x !== id); persist(); },
    save_settings: ({ settings }) => { settings.badge.opacity = Math.min(1, Math.max(0.3, settings.badge.opacity)); state.settings = settings; persist(); },
    reset_badge_positions: () => persist(),
    discard_drafts: () => { for (const k in drafts) delete drafts[k]; },
    retry_person: () => persist(),
    open_settings: ({ tab } = {}) => { if (page === "settings" && tab) emit("open-tab", tab); },
    hide_popup: () => {},
    quit_app: () => {},
  };

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args = {}) => {
        log.push([cmd, JSON.parse(JSON.stringify(args))]);
        if (!commands[cmd]) throw `unknown command ${cmd}`;
        try { return await commands[cmd](args); } catch (e) { throw String(e); }
      },
    },
    event: { listen: async (name, cb) => { (listeners[name] ||= []).push(cb); return () => {}; } },
    window: { getCurrentWindow: () => ({ label }) },
    app: { getVersion: async () => "0.1.0-harness" },
  };
  window.__harness = { reset: () => { localStorage.removeItem(KEY); state = load(); persist(); }, state: () => state };
})();
