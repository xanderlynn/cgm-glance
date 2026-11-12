# CGM Glance: plan and execution review

**Status:** proposal, no code yet. Prototypes are in [`prototypes/`](prototypes/).

## 1. Goal

A small desktop app for Windows, macOS and Linux that shows the current Dexcom glucose
reading for one or more people with T1D:

- in the **system tray / menu bar** (bottom-right on Windows), as an icon that carries the
  number and a color for the range, with a trend arrow where space allows
- optionally as a small **floating desktop widget** that stays on top
- with a **safe, guided way to add people**, where passwords never land in a plain file

It is a glance tool. It is **not** a medical device and does not replace the Dexcom app's
alarms. The UI says so in the setup wizard and the About screen.

## 2. Where the data comes from

| Source | Real time? | What the user gives the app | Verdict |
|---|---|---|---|
| **Nightscout** (if the person already runs one) | Yes, every 5 min | Site URL + a **read-only access token** | **Safest. Recommended when available.** No Dexcom password involved, token can be revoked any time. |
| **Dexcom Share** (the API behind the Follow app; used by pydexcom, Nightscout bridge, xDrip+) | Yes, every 5 min | Dexcom login of the **person wearing the sensor** (the "publisher", not a follower), plus region (US / outside US / Japan) | **Main path for most families.** Unofficial and undocumented, so it can change. Share must be on with at least one follower. |
| **Official Dexcom API** (developer.dexcom.com, OAuth) | No. 1 h delay in the US, 3 h elsewhere | OAuth sign-in in the browser | **Not for v1.** Cleanest auth, but the delay makes it useless for a live tray number. Revisit if Dexcom opens real-time access to individuals. |

The code has one `GlucoseSource` interface, so a source can be added or swapped without
touching the tray, popup or widget.

## 3. Recommended stack

**Tauri 2** (Rust core + a small web UI for the popup, widget and settings windows).

- Small: about 5 to 10 MB installers and low idle memory, since it uses the OS webview
  instead of bundling Chromium (Electron would be 100 MB+).
- First-class tray API on all three OSes, plus always-on-top frameless windows for the widget.
- Rust side owns all network and secret handling; the web UI never sees a password.
- Mature plugins for autostart, single-instance, notifications and signed auto-updates.
- One GitHub Actions matrix builds `.msi`/`.exe`, `.dmg` (universal) and `.AppImage`/`.deb`.

Alternatives considered: Electron (too heavy for a tray app), Python + pystray (packaging and
signing are painful, no good widget story), Go + systray (fine tray, weak widget/settings UI).

### How the number gets into the tray on each OS

| OS | What shows | How |
|---|---|---|
| **Windows 10/11** | Icon with the number drawn into it, colored by range; tooltip with name, value, arrow, age | Render a 16/20/24/32 px RGBA icon at runtime (tiny-skia + an embedded condensed font) matched to the display scale. Windows tray cannot show text next to an icon. |
| **macOS** | Text in the menu bar: `● 142 →` (multiple people: `A 142 → · M 88 ↘`) | `tray.set_title()` beside a small colored dot icon. Most legible of the three. |
| **Linux** | Icon + text label | AppIndicator label (KDE, XFCE, Cinnamon, GNOME with the AppIndicator extension). No tooltips on Linux, so the popup carries the details. |

The "Windows desktop widget" in the Windows 11 Widgets Board needs MSIX packaging and
Adaptive Cards and only works on Windows 11. v1 ships a **floating mini widget** instead
(works on every OS); the Widgets Board can be a later phase if wanted.

## 4. Adding people safely

Wizard in four steps (see `prototypes/03-add-person.svg`):

1. **Choose a source.** Nightscout is listed first and marked recommended. Dexcom Share
   explains in plain words that it needs the wearer's own Dexcom login.
2. **Enter details.** Region picker for Share. Password field never echoes back, can't be
   copied out, and is held only in memory until saved.
3. **Test the connection.** The app signs in once and shows the latest reading and its age.
   The user confirms "Yes, that's Alex" before anything is saved. Wrong person or wrong
   region gets caught here.
4. **Display options.** Name, initials, color, units (mg/dL or mmol/L), target range,
   own tray icon or not.

Security rules:

- **Secrets go only into the OS keychain**: Windows Credential Manager, macOS Keychain,
  Linux Secret Service (GNOME Keyring / KWallet) via the `keyring` crate. If no keychain is
  available on Linux, the app refuses to save and explains how to install one. It does not
  fall back to a plain file.
- The config file (`config.json` in the OS config dir) holds only non-secret settings and a
  random id per person that points at the keychain entry.
- The Dexcom session id is kept in memory only.
- Removing a person deletes their keychain entry.
- Passwords and tokens never pass into the web UI; the UI calls a Rust command
  `save_person(...)` once, and Rust stores the secret and drops it.
- **Lockout protection:** on a "wrong password" reply the app stops polling that person and
  marks them "Sign in again" instead of retrying. Dexcom locks accounts after repeated
  failures.
- Network: HTTPS only, to the Dexcom Share host for the chosen region or the user's own
  Nightscout URL. Nothing else. No telemetry, analytics or crash upload.
- Logs redact usernames, tokens and session ids.
- Tauri locked down: strict CSP, no remote content in webviews, minimal IPC allowlist.

## 5. Behavior details that matter for safety

- **Stale data is obvious.** Over 10 minutes old: grey icon, age badge. Over 20 minutes:
  `---` instead of the old number. The app never shows an old number as if it were current.
- **Polling** follows the sensor: next fetch at last reading time + 5 min 10 s, with
  backoff on network errors. One request per person per cycle.
- **Ranges** default to 70 to 180 mg/dL (3.9 to 10.0 mmol/L) with urgent low at 55,
  user-adjustable per person. Colors are paired with shape/arrow so color is not the only cue.
- **mmol/L in a 16 px Windows icon:** `10.4` does not fit legibly. Plan: show `10` with a
  small decimal, and full value in the tooltip. Needs a legibility test in M1.
- **Optional notifications** (off by default): urgent low, data stale for 20+ min,
  sign-in needed.
- **Multiple people:** one tray icon per person (each can be turned off), or a single
  icon showing a chosen primary person. Popup and widget always list everyone.

## 6. Milestones

| # | Deliverable | Done when |
|---|---|---|
| M0 | Repo, plan, prototypes (this) | Plan approved |
| M1 | Tauri skeleton, tray icon renderer, fake data source | Icon legible at 100/125/150% on Windows; title works on macOS and KDE |
| M2 | Dexcom Share + Nightscout clients | Unit tests against recorded (scrubbed) responses; live test with a real account |
| M3 | Add-person wizard + keychain storage | Secret never appears in config, logs or webview (tested) |
| M4 | Tray popup + floating widget | Matches prototypes; widget position remembered |
| M5 | CI builds and releases for 3 OSes | Installers attached to a GitHub Release |
| M6 | Hardening: autostart, notifications, signed updater, accessibility | Checklist in section 7 passes |

## 7. Execution review: risks and how each is handled

| Risk | Impact | Mitigation |
|---|---|---|
| Dexcom changes the unofficial Share API | Readings stop | Source abstraction; Nightscout path; clear "can't reach Dexcom" state; quick patch release via updater |
| Storing the wearer's full Dexcom login | Account exposure if the PC is compromised | OS keychain only; Nightscout recommended; explain trade-off in wizard |
| Account lockout from retries | Wearer locked out of their own app | Stop on auth failure, no blind retries |
| Showing old data as current | Wrong decision | Stale thresholds, grey + `---`, age always visible |
| Number too small in Windows tray | Unreadable | Runtime render per DPI, condensed font, legibility test in M1, color-dot fallback mode |
| GNOME hides tray icons | Nothing visible on stock GNOME | Detect and show a one-time hint about the AppIndicator extension; widget still works |
| Unsigned builds | SmartScreen / Gatekeeper warnings | Decide on signing (Apple Developer $99/yr; Windows cert or Azure Trusted Signing). Unsigned is fine for personal use |
| Medical/legal perception | Misuse | "Not a medical device" in wizard, About and README; no dosing advice anywhere |

## 8. Decisions needed before M1

1. Data source in practice: Dexcom Share, Nightscout, or both?
2. Units: mg/dL, mmol/L, or per person?
3. Tray layout with several people: one icon each, or one primary icon?
4. Code signing: unsigned personal builds, or pay for signing?
5. App name (working name: **CGM Glance**) and whether the repo stays private.
