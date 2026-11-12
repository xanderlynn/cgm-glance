"""Profiles CGM Glance (release, --demo) under different settings. macOS only.

Usage (from the repo root, after `cargo build --release` in src-tauri):
    python3 tests/perf/matrix.py src-tauri/target/release/cgm-glance 75
    python3 tests/perf/matrix.py src-tauri/target/release/cgm-glance 600 soak
Run once with --demo first so the demo settings file exists. Quit any running copy first.
Keep the screen unlocked: hidden windows skip work, which would make results look better.
For each variant: writes the demo config, launches the app, samples CPU time and RSS of the
app and the WebKit processes that belong to it every 5 s, then quits it.
`soak` runs one long variant with badge recycling every 3 minutes instead.
"""
import copy, json, os, subprocess, sys, time

binary, seconds = sys.argv[1], int(sys.argv[2])
soak = len(sys.argv) > 3 and sys.argv[3] == "soak"
CFG = os.path.expanduser("~/Library/Application Support/io.github.cgmglance/config.demo.json")
base = json.load(open(CFG))
for p in base["people"]:
    p["units"] = "mgdl"

def ps():
    out = subprocess.run(["ps", "-A", "-o", "pid=,rss=,time=,comm="], capture_output=True, text=True).stdout
    procs = {}
    for line in out.splitlines():
        parts = line.split(None, 3)
        if len(parts) == 4:
            secs = 0.0
            for x in parts[2].split(":"):
                secs = secs * 60 + float(x)
            procs[int(parts[0])] = (int(parts[1]), secs, parts[3].split("/")[-1])
    return procs

def variant(name, badge=None, tray=None, fast=True, mmol=False, settings_open=False, env=None):
    cfg = copy.deepcopy(base)
    cfg["settings"]["demo_fast"] = fast
    cfg["settings"]["badge"].update(badge or {"enabled": False})
    cfg["settings"]["tray"].update(tray or {"mode": "per_person"})
    for p in cfg["people"]:
        p["units"] = "mmol" if mmol else "mgdl"
    cfg["badge_positions"] = {}
    json.dump(cfg, open(CFG, "w"), indent=2)

    before = {p for p, (_, _, c) in ps().items() if "WebKit" in c}
    app = subprocess.Popen([binary, "--demo"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                           env={**os.environ, **(env or {})})
    time.sleep(4)
    if settings_open:
        subprocess.run([binary, "--demo"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=20)
    time.sleep(4)
    samples = []  # (t, total_rss_kb, total_cpu_s, per-process rss dict)
    start = time.time()
    seen_cpu = {}  # cpu seconds per pid at last sight, so processes that exit still count
    first_cpu = None
    while time.time() - start < seconds:
        procs = ps()
        ours = [app.pid] + [p for p, (_, _, c) in procs.items() if "WebKit" in c and p not in before]
        ours = [p for p in ours if p in procs]
        for p in ours:
            seen_cpu[p] = procs[p][1]
        total_cpu = sum(seen_cpu.values())
        if first_cpu is None:
            first_cpu = total_cpu
        samples.append((time.time() - start, sum(procs[p][0] for p in ours), total_cpu,
                        {p: (procs[p][2], procs[p][0]) for p in ours}))
        time.sleep(5)
    app.terminate()
    app.wait(timeout=10)
    time.sleep(2)

    t0, r0, c0, _ = samples[0]
    t1, r1, c1, _ = samples[-1]
    cpu = (c1 - c0) / max(t1 - t0, 1) * 100
    # Second-half growth rate of total memory, MB per minute.
    half = [s for s in samples if s[0] >= t1 / 2]
    slope = (half[-1][1] - half[0][1]) / 1024 / max(half[-1][0] - half[0][0], 1) * 60
    windows = sum(1 for c, _ in samples[-1][3].values() if c.endswith("WebContent"))
    print(f"{name:44} windows {windows}  CPU {cpu:4.2f}%  RAM {r0/1024:6.1f} -> {r1/1024:6.1f} MB"
          f"  2nd-half {slope:+5.2f} MB/min", flush=True)
    return samples

if soak:
    s = variant("soak: pill separate + settings, recycle 3 min",
                {"enabled": True, "style": "pill", "combined": False}, settings_open=True,
                env={"CGM_GLANCE_RECYCLE_SECS": "180"})
    print("total RAM (MB) each minute:", " ".join(f"{x[1]/1024:.0f}" for x in s if int(x[0]) % 60 < 5))
    sys.exit()

variant("badges off, tray per person")
variant("pill, separate, M, top, free", {"enabled": True, "style": "pill", "combined": False, "size": "m",
        "layer": "top", "position": "free"})
variant("ring, combined, L, behind windows, right edge", {"enabled": True, "style": "ring", "combined": True,
        "size": "l", "layer": "bottom", "position": "edge", "edge": "right"})
variant("faces, S, normal, corner, locked, click-through", {"enabled": True, "style": "faces", "combined": False,
        "size": "s", "layer": "normal", "position": "corner", "corner": "bottom_left", "lock": True,
        "click_through": True})
variant("one tray icon, mmol/L, pill", {"enabled": True, "style": "pill", "combined": False},
        tray={"mode": "primary"}, mmol=True)
variant("normal 5-min cadence (not fast), pill", {"enabled": True, "style": "pill", "combined": False}, fast=False)
variant("settings window open, pill", {"enabled": True, "style": "pill", "combined": False}, settings_open=True)
