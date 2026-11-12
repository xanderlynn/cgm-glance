# CGM Glance

A small tray / menu-bar app for Windows, macOS and Linux that shows the current Dexcom
glucose reading for one or more people with T1D, plus an optional movable desktop badge.

> **Not a medical device.** CGM Glance shows readings at a glance. It does not replace the
> Dexcom app or its alarms. Never make treatment decisions from it alone.

## What it does

- **Tray icon:** on Windows, the number is drawn inside the icon, colored by range, with a
  notch for the trend. On macOS and Linux it's a colored dot with the number beside it.
  Hover (Windows) or click to see everyone.
- **Desktop badge (optional):** turn it on in Settings → Desktop badge. Pick a style:
  - **Pill:** small dark capsule with a colored dot, the number and the trend.
  - **Ring:** the number inside a colored circle.
  - **Faces:** a discreet emoji instead of the number (🙂 in range, 😬 high, 😟 low,
    😱 urgent low, 😴 old reading). Hover the badge or open the tray to see the number.

  Also choose size, layer (always on top, normal, behind windows), and position (drag
  freely, snap to an edge, or a corner). It can be locked in place.
- **Several people:** one tray icon each or one combined icon; one badge each or combined.
- **Old data is obvious:** grey after 10 minutes, the number is hidden after 20.

## Data sources

| Source | Needs | Notes |
|---|---|---|
| Nightscout | Site address + read-only token | Recommended. No Dexcom password. |
| Dexcom Share | Login of the person wearing the sensor + region | Unofficial API used by the Follow app. Share must be on. |
| Demo data | Nothing | Synthetic readings for testing. Always marked DEMO. |

Sign-in details are stored only in the system keychain. See [SECURITY.md](SECURITY.md).

## Try it without an account (test mode)

```bash
cd src-tauri
cargo run -- --demo
```

`--demo` uses a separate settings file, adds three demo people (one moving through all
ranges, one low, one with an old reading), turns on the badge, and makes a new reading
arrive every 10 seconds. It never touches your real settings or keychain.

You can also add a **Demo data** person from the normal app. Scenarios cover every state:
steady, high, low, urgent low, old reading, no data, rejected password, and no internet.

### UI test harness

Runs the real popup, badge and settings pages in a normal browser with a fake backend
and synthetic data, for checking layouts and flows quickly:

```bash
python3 tests/ui-harness/serve.py
# then open http://localhost:8765/tests/ui-harness/  (everything at once)
#       or http://localhost:8765/tests/ui-harness/badges.html  (every badge style)
```

### Performance tests (macOS)

`tests/perf/matrix.py` runs the release build under each badge style and tray mode and
reports CPU and memory. See the `performance-test` issues for the latest results.

## Build from source

Needs [Rust](https://rustup.rs) and, on Linux:
`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev pkg-config`.

```bash
cd src-tauri
cargo test            # unit tests (synthetic data only)
cargo run             # run the app
cargo install tauri-cli --version "^2" --locked
cargo tauri build     # installers in src-tauri/target/release/bundle
```

Releases: push a tag like `v0.1.0` and GitHub Actions builds installers for all three
systems into a draft release. Builds are not code-signed, so Windows SmartScreen and macOS
Gatekeeper will warn on first launch.

## Layout

```
src-tauri/src/
  main.rs        app start, --demo flag
  model.rs       readings, ranges, stale rules, formatting
  config.rs      non-secret settings file
  secrets.rs     OS keychain
  sources/       dexcom share, nightscout, demo
  poller.rs      fetches on the sensor's 5-minute schedule
  icon.rs        draws tray icons (no font files)
  ui.rs          tray, popup, settings window, desktop badges
  commands.rs    calls from the windows
ui/              popup, badge and settings pages (plain HTML/JS)
docs/            plan and prototypes
```

## Known limits

- Linux on Wayland decides window position and stacking, so badge position and layer may
  not apply. Stock GNOME needs the AppIndicator extension to show tray icons.
- Tray click events aren't available on Linux; use the tray menu's "Show readings".
- Dexcom Share is unofficial and may change without notice.

## Prototypes

See [docs/PLAN.md](docs/PLAN.md) and [docs/prototypes](docs/prototypes).

## License

MIT
