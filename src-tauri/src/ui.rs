//! Tray icons, the popup, the settings window and the desktop badges.

use crate::config::{
    BadgeSettings, BadgeSize, BadgeStyle, Corner, Edge, IconStyle, Layer, PositionMode, SavedPos,
    TrayMode,
};
use crate::icon::{dot_icon, number_icon, IconSpec};
use crate::model::Status;
use crate::state::{self, AppState, Geometry, PersonView, Snapshot};
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};

/// Rebuilds the view model and pushes it to the tray, the badges and every open window,
/// but only when something visible changed (the ticker calls this every 5 s).
pub fn refresh(app: &AppHandle) {
    push(app, false);
}

/// Like `refresh`, but always re-applies tray and badge state (e.g. after resetting
/// badge positions, which isn't part of the snapshot).
pub fn refresh_force(app: &AppHandle) {
    push(app, true);
}

fn push(app: &AppHandle, force: bool) {
    crate::alarms::check(app);
    let state = app.state::<AppState>();
    let snap = state::snapshot(&state);
    // The tray icon size is part of the key so a display-scale change redraws it.
    let key = format!(
        "{}|{}",
        tray_icon_size(app),
        serde_json::to_string(&snap).unwrap_or_default()
    );
    {
        let mut inner = state.lock();
        if !force && inner.last_pushed == key {
            return;
        }
        inner.last_pushed = key;
    }
    let app2 = app.clone();
    let snap2 = snap.clone();
    let _ = app.run_on_main_thread(move || {
        update_trays(&app2, &snap2);
        sync_badges(&app2, &snap2);
    });
    // Only a tiny signal goes out as an event. Tauri delivers events by running a script in
    // each window, and WebKit leaks memory per script in proportion to its size, so the
    // windows fetch the snapshot themselves through the request channel instead.
    let _ = app.emit("changed", ());
}

/// How often badge windows are recreated. WebKit window processes slowly hold on to memory
/// over many updates; a fresh window resets that, so memory stays bounded however long the
/// app runs. `CGM_GLANCE_RECYCLE_SECS` overrides it (used by the soak test).
fn recycle_interval() -> Duration {
    let secs = std::env::var("CGM_GLANCE_RECYCLE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2 * 3600)
        .max(60);
    Duration::from_secs(secs)
}

/// Closes all badge windows; the next refresh recreates them in the same places.
fn recycle_badges(app: &AppHandle) {
    for (label, w) in app.webview_windows() {
        if label.starts_with("badge-") {
            let _ = w.destroy();
        }
    }
    let state = app.state::<AppState>();
    let mut inner = state.lock();
    inner.badge_geometry.clear();
    inner.badge_flags.clear();
}

/// Blinks the tray icons of people with an unacknowledged flashing alarm. Runs only while
/// one exists, and touches only the tray (windows animate their own flashing with CSS).
pub fn ensure_flasher(app: &AppHandle) {
    let state = app.state::<AppState>();
    {
        let mut inner = state.lock();
        if inner.flasher_running {
            return;
        }
        inner.flasher_running = true;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(600)).await;
            let state = app.state::<AppState>();
            let flashing = {
                let mut inner = state.lock();
                let flashing = crate::alarms::any_flashing(&inner);
                inner.flash_dim = flashing && !inner.flash_dim;
                inner.flasher_running = flashing;
                flashing
            };
            let snap = state::snapshot(&state);
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || update_trays(&app2, &snap));
            if !flashing {
                break;
            }
        }
    });
}

/// Checks every 5 s so ages and stale states update between fetches.
pub fn start_ticker(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let every = recycle_interval();
        let mut last_recycle = Instant::now();
        loop {
            if last_recycle.elapsed() >= every {
                last_recycle = Instant::now();
                let app2 = app.clone();
                let _ = app.run_on_main_thread(move || {
                    recycle_badges(&app2);
                    refresh_force(&app2);
                });
            }
            // Short interval: after the computer wakes from sleep, old readings must turn
            // grey quickly instead of looking current.
            tokio::time::sleep(Duration::from_secs(5)).await;
            app.state::<AppState>()
                .lock()
                .drafts
                .retain(|_, d| d.created.elapsed() < Duration::from_secs(10 * 60));
            refresh(&app);
        }
    });
}

// ---------- Tray ----------

fn tray_icon_size(app: &AppHandle) -> u32 {
    if cfg!(windows) {
        // Draw at the exact pixel size Windows uses, so digits stay crisp.
        let scale = app
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| m.scale_factor())
            .unwrap_or(1.0);
        (16.0 * scale).round() as u32
    } else {
        44
    }
}

fn short_title(v: &PersonView) -> String {
    match (&v.value, v.status) {
        (_, Status::SignIn) => "!".into(),
        // An old reading always says how old it is.
        (Some(val), Status::Stale) => format!("{val} · {}m", v.age_min.unwrap_or(0)),
        (Some(val), _) if v.arrow.is_empty() => val.clone(),
        (Some(val), _) => format!("{val} {}", v.arrow),
        _ => "---".into(),
    }
}

/// Lower is more urgent. Used to pick which person colors a shared tray icon.
fn urgency(s: Status) -> u8 {
    match s {
        Status::UrgentLow => 0,
        Status::Low => 1,
        Status::High => 2,
        Status::SignIn | Status::Error | Status::NoData => 3,
        Status::Stale => 4,
        Status::Loading => 5,
        Status::InRange => 6,
    }
}

fn update_trays(app: &AppHandle, snap: &Snapshot) {
    let tray = &snap.settings.tray;
    let mut desired: Vec<(String, Vec<&PersonView>)> = vec![];
    if snap.people.is_empty() {
        desired.push(("main".into(), vec![]));
    } else if tray.mode == TrayMode::PerPerson {
        for (v, p) in snap.people.iter().zip(&snap.config_people) {
            if p.tray_icon {
                desired.push((format!("p-{}", v.id), vec![v]));
            }
        }
    }
    if desired.is_empty() {
        let mut list: Vec<&PersonView> = snap.people.iter().collect();
        if let Some(pid) = &tray.primary {
            list.sort_by_key(|v| &v.id != pid);
        }
        if tray.mode == TrayMode::Single {
            // Only the chosen person (or the first, if none is chosen or they were removed).
            list.truncate(1);
        }
        desired.push(("main".into(), list));
    }

    let size = tray_icon_size(app);
    let dim_phase = app.state::<AppState>().lock().flash_dim;
    let use_dot = !cfg!(windows) || tray.icon_style == IconStyle::Dot;
    for (id, list) in &desired {
        // A shared icon shows whoever needs attention most (main person on ties).
        // In single mode the list holds just the chosen person.
        let first = list.iter().min_by_key(|v| urgency(v.status));
        let spec = IconSpec {
            status: first.map(|v| v.status).unwrap_or(Status::NoData),
            text: first.and_then(|v| v.icon_text.as_deref()),
            direction: first.map(|v| v.direction).unwrap_or(0),
            demo: first.is_some_and(|v| v.is_demo),
        };
        let mut tooltip = if list.is_empty() {
            "CGM Glance: add a person in Settings".to_string()
        } else {
            list.iter()
                .map(|v| state::summary_line(v))
                .collect::<Vec<_>>()
                .join("\n")
        };
        if tooltip.chars().count() > 120 {
            tooltip = tooltip.chars().take(119).collect::<String>() + "…";
        }
        // Titles stay short: macOS hides menu-bar items that don't fit (e.g. beside the
        // notch). A shared icon names only the person it is colored for; the tooltip and
        // popup list everyone.
        let several = snap.people.len() > 1 && tray.show_initials;
        let title = match first {
            None => String::new(),
            Some(v) if several => format!("{} {}", v.initials, short_title(v)),
            Some(v) => short_title(v),
        };
        // An unacknowledged alarm makes the icon blink between normal and dimmed.
        let dim = dim_phase
            && list
                .iter()
                .any(|v| v.alarm.as_ref().is_some_and(|a| a.flash));
        // Skip trays whose icon, tooltip and title are unchanged.
        let look = format!(
            "{:?}|{:?}|{}|{}|{size}|{use_dot}|{dim}|{tooltip}|{title}",
            spec.status, spec.text, spec.direction, spec.demo
        );
        let existing = app.tray_by_id(id);
        let state = app.state::<AppState>();
        if existing.is_some() && state.lock().tray_looks.get(id) == Some(&look) {
            continue;
        }
        let mut rgba = if use_dot {
            dot_icon(&spec, size)
        } else {
            number_icon(&spec, size)
        };
        if dim {
            for px in rgba.chunks_exact_mut(4) {
                px[3] /= 5;
            }
        }
        let image = Image::new_owned(rgba, size, size);
        if let Some(t) = existing {
            let _ = t.set_icon(Some(image));
            let _ = t.set_tooltip(Some(&tooltip));
            if !cfg!(windows) {
                let _ = t.set_title(Some(&title));
            }
        } else if let Err(e) = build_tray(app, id, image, &tooltip, &title) {
            eprintln!("cgm-glance: tray icon failed: {e}");
            continue;
        }
        state.lock().tray_looks.insert(id.clone(), look);
    }

    let keep: Vec<String> = desired.into_iter().map(|(id, _)| id).collect();
    let state = app.state::<AppState>();
    let old: Vec<String> = {
        let mut inner = state.lock();
        let old: Vec<String> = inner
            .trays
            .iter()
            .filter(|id| !keep.contains(id))
            .cloned()
            .collect();
        inner.trays = keep.into_iter().collect();
        for id in &old {
            inner.tray_looks.remove(id);
        }
        old
    };
    for id in old {
        // Hide first: dropping a removed tray icon doesn't reliably take it off the menu bar.
        if let Some(t) = app.tray_by_id(&id) {
            let _ = t.set_visible(false);
        }
        let _ = app.remove_tray_by_id(&id);
    }
}

fn build_tray(
    app: &AppHandle,
    id: &str,
    image: Image<'static>,
    tooltip: &str,
    title: &str,
) -> tauri::Result<()> {
    let menu = MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id("show", "Show readings").build(app)?)
        .item(&MenuItemBuilder::with_id("settings", "Settings…").build(app)?)
        .item(&MenuItemBuilder::with_id("badge", "Show / hide desktop badge").build(app)?)
        .item(&MenuItemBuilder::with_id("ack", "Acknowledge alarms").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id("quit", "Quit CGM Glance").build(app)?)
        .build()?;
    let mut b = TrayIconBuilder::with_id(id)
        .icon(image)
        .tooltip(tooltip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| handle_menu(app, ev.id().as_ref()))
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = ev
            {
                toggle_popup(tray.app_handle(), Some(rect));
            }
        });
    if !cfg!(windows) {
        b = b.title(title);
    }
    b.build(app)?;
    Ok(())
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => show_popup(app, None),
        "settings" => open_settings(app, None),
        "badge" => {
            let state = app.state::<AppState>();
            {
                let mut inner = state.lock();
                inner.config.settings.badge.enabled = !inner.config.settings.badge.enabled;
                state.save(&inner);
            }
            refresh(app);
        }
        "ack" => {
            crate::alarms::acknowledge(app, None);
            refresh(app);
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

// ---------- Popup ----------

const POPUP_W: f64 = 380.0;

pub fn toggle_popup(app: &AppHandle, rect: Option<tauri::Rect>) {
    if let Some(w) = app.get_webview_window("popup") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
            return;
        }
    }
    // The click that took focus away from the popup also lands here; don't reopen it.
    let recently_hidden = app
        .state::<AppState>()
        .lock()
        .popup_hidden_at
        .is_some_and(|t| t.elapsed() < Duration::from_millis(400));
    if !recently_hidden {
        show_popup(app, rect);
    }
}

pub fn show_popup(app: &AppHandle, rect: Option<tauri::Rect>) {
    let count = app.state::<AppState>().lock().config.people.len().max(1) as f64;
    let height = (128.0 + 140.0 * count).min(660.0);
    let win = match app.get_webview_window("popup") {
        Some(w) => w,
        None => {
            let w =
                match WebviewWindowBuilder::new(app, "popup", WebviewUrl::App("popup.html".into()))
                    .title("CGM Glance")
                    .inner_size(POPUP_W, height)
                    .decorations(false)
                    .resizable(false)
                    .skip_taskbar(true)
                    .always_on_top(true)
                    .visible(false)
                    .build()
                {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("cgm-glance: popup failed: {e}");
                        return;
                    }
                };
            let app2 = app.clone();
            let w2 = w.clone();
            w.on_window_event(move |ev| {
                if let WindowEvent::Focused(false) = ev {
                    let _ = w2.hide();
                    app2.state::<AppState>().lock().popup_hidden_at = Some(Instant::now());
                }
            });
            w
        }
    };
    let _ = win.set_size(LogicalSize::new(POPUP_W, height));

    let anchor = rect.map(|r| {
        let p = r.position.to_physical::<f64>(1.0);
        let s = r.size.to_physical::<f64>(1.0);
        (p.x, p.y, s.width, s.height)
    });
    let monitor = match anchor {
        Some((x, y, _, _)) => app.monitor_from_point(x, y).ok().flatten(),
        None => None,
    }
    .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let wa = m.work_area();
        let (pw, ph) = (POPUP_W * scale, height * scale);
        let (left, top) = (wa.position.x as f64, wa.position.y as f64);
        let (right, bottom) = (left + wa.size.width as f64, top + wa.size.height as f64);
        let gap = 8.0 * scale;
        let (x, y) = match anchor {
            Some((ax, ay, aw, ah)) => {
                let x = ax + aw / 2.0 - pw / 2.0;
                let mid = m.position().y as f64 + m.size().height as f64 / 2.0;
                let y = if ay > mid {
                    ay - ph - gap
                } else {
                    ay + ah + gap
                };
                (x, y)
            }
            None if cfg!(target_os = "macos") => (right - pw - gap, top + gap),
            None => (right - pw - gap, bottom - ph - gap),
        };
        let x = x.clamp(left + gap, (right - pw - gap).max(left));
        let y = y.clamp(top + gap, (bottom - ph - gap).max(top));
        let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }
    let _ = win.show();
    let _ = win.set_focus();
}

// ---------- Settings ----------

pub fn open_settings(app: &AppHandle, tab: Option<&str>) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(t) = tab {
            let _ = app.emit_to("settings", "open-tab", t);
        }
        return;
    }
    let demo = app.state::<AppState>().demo_mode;
    let url = match tab {
        Some(t) => format!("settings.html#{t}"),
        None => "settings.html".into(),
    };
    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App(url.into()))
        .title(if demo {
            "CGM Glance Settings (demo mode)"
        } else {
            "CGM Glance Settings"
        })
        .inner_size(840.0, 680.0)
        .min_inner_size(660.0, 540.0)
        .center()
        .build();
    if let Err(e) = built {
        eprintln!("cgm-glance: settings window failed: {e}");
    }
}

// ---------- Desktop badges ----------

pub fn badge_logical_size(style: BadgeStyle, size: BadgeSize, count: usize) -> (f64, f64) {
    let k = match size {
        BadgeSize::S => 0.8,
        BadgeSize::M => 1.0,
        BadgeSize::L => 1.3,
    };
    let (w, h): (f64, f64) = match style {
        BadgeStyle::Pill => (156.0, 34.0),
        BadgeStyle::Ring => (64.0, 64.0),
        BadgeStyle::Faces => (64.0, 64.0),
    };
    let n = count.max(1) as f64;
    // 8 px around each badge leaves room for its shadow.
    (
        (w * k).round() + 16.0,
        ((h * k).round() * n + 8.0 * (n - 1.0)).round() + 16.0,
    )
}

struct Area {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Area {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// Where badge `i` of `n` goes for Edge / Corner modes (physical pixels, window top-left).
fn placed(b: &BadgeSettings, wa: &Area, i: i32, n: i32, pw: i32, ph: i32, m: i32) -> (i32, i32) {
    let gap = 0; // windows already include padding
    let (right, bottom) = (wa.x + wa.w, wa.y + wa.h);
    match b.position {
        PositionMode::Edge => match b.edge {
            Edge::Right | Edge::Left => {
                let total = n * ph + (n - 1) * gap;
                let x = if b.edge == Edge::Right {
                    right - pw - m
                } else {
                    wa.x + m
                };
                (x, wa.y + (wa.h - total) / 2 + i * (ph + gap))
            }
            Edge::Top | Edge::Bottom => {
                let total = n * pw + (n - 1) * gap;
                let y = if b.edge == Edge::Bottom {
                    bottom - ph - m
                } else {
                    wa.y + m
                };
                (wa.x + (wa.w - total) / 2 + i * (pw + gap), y)
            }
        },
        _ => {
            let corner = if b.position == PositionMode::Corner {
                b.corner
            } else {
                Corner::TopRight
            };
            let x = match corner {
                Corner::TopLeft | Corner::BottomLeft => wa.x + m,
                _ => right - pw - m,
            };
            let y = match corner {
                Corner::TopLeft | Corner::TopRight => wa.y + m + i * (ph + gap),
                _ => bottom - m - (i + 1) * ph - i * gap,
            };
            (x, y)
        }
    }
}

fn sync_badges(app: &AppHandle, snap: &Snapshot) {
    let b = &snap.settings.badge;
    let selected: Vec<&PersonView> = snap
        .people
        .iter()
        .filter(|v| b.people.is_empty() || b.people.contains(&v.id))
        .collect();
    let desired: Vec<(String, usize)> = if !b.enabled || selected.is_empty() {
        vec![]
    } else if b.combined {
        vec![("badge-all".into(), selected.len())]
    } else {
        selected
            .iter()
            .map(|v| (format!("badge-{}", v.id), 1))
            .collect()
    };

    for (label, w) in app.webview_windows() {
        if label.starts_with("badge-") && !desired.iter().any(|(l, _)| *l == label) {
            let _ = w.destroy();
            let state = app.state::<AppState>();
            let mut inner = state.lock();
            inner.badge_geometry.remove(&label);
            inner.badge_flags.remove(&label);
        }
    }
    if desired.is_empty() {
        return;
    }

    let Some(mon) = app.primary_monitor().ok().flatten() else {
        return;
    };
    let scale = mon.scale_factor();
    let wa = mon.work_area();
    let area = Area {
        x: wa.position.x,
        y: wa.position.y,
        w: wa.size.width as i32,
        h: wa.size.height as i32,
    };
    let margin = (8.0 * scale).round() as i32;
    let n = desired.len() as i32;
    let on_screen: Vec<Area> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let wa = m.work_area();
            Area {
                x: wa.position.x,
                y: wa.position.y,
                w: wa.size.width as i32,
                h: wa.size.height as i32,
            }
        })
        .collect();
    let flags = format!("{:?}|{}|{}", b.layer, b.click_through, b.lock);

    for (i, (label, count)) in desired.iter().enumerate() {
        let (lw, lh) = badge_logical_size(b.style, b.size, *count);
        let (pw, ph) = ((lw * scale).round() as i32, (lh * scale).round() as i32);
        let state = app.state::<AppState>();
        let (target, flags_changed, geometry_changed) = {
            let mut inner = state.lock();
            let (x, y) = match (b.position, inner.config.badge_positions.get(label)) {
                // A saved spot on a monitor that's gone falls back to the default place.
                (PositionMode::Free, Some(p)) if on_screen.iter().any(|r| r.contains(p.x, p.y)) => {
                    (p.x, p.y)
                }
                _ => placed(b, &area, i as i32, n, pw, ph, margin),
            };
            let g = Geometry {
                w: pw as u32,
                h: ph as u32,
                x,
                y,
            };
            let geometry_changed = inner.badge_geometry.get(label) != Some(&g);
            let flags_changed = inner.badge_flags.get(label) != Some(&flags);
            // Record before moving, so the Moved event from our own move is ignored.
            inner.badge_geometry.insert(label.clone(), g);
            inner.badge_flags.insert(label.clone(), flags.clone());
            (g, flags_changed, geometry_changed)
        };

        let (win, created) = match app.get_webview_window(label) {
            Some(w) => (w, false),
            None => match create_badge(app, label, lw, lh) {
                Some(w) => (w, true),
                None => continue,
            },
        };
        if geometry_changed || created {
            let _ = win.set_size(PhysicalSize::new(target.w, target.h));
            let _ = win.set_position(PhysicalPosition::new(target.x, target.y));
        }
        if flags_changed || created {
            match b.layer {
                Layer::Top => {
                    let _ = win.set_always_on_bottom(false);
                    let _ = win.set_always_on_top(true);
                }
                Layer::Normal => {
                    let _ = win.set_always_on_top(false);
                    let _ = win.set_always_on_bottom(false);
                }
                Layer::Bottom => {
                    let _ = win.set_always_on_top(false);
                    let _ = win.set_always_on_bottom(true);
                }
            }
            let _ = win.set_ignore_cursor_events(b.click_through);
        }
        if created {
            let _ = win.show();
        }
    }
}

fn create_badge(app: &AppHandle, label: &str, lw: f64, lh: f64) -> Option<tauri::WebviewWindow> {
    let w = WebviewWindowBuilder::new(app, label, WebviewUrl::App("badge.html".into()))
        .title("CGM Glance badge")
        .inner_size(lw, lh)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()
        .map_err(|e| eprintln!("cgm-glance: badge window failed: {e}"))
        .ok()?;
    #[cfg(target_os = "macos")]
    let _ = w.set_visible_on_all_workspaces(true);
    let app2 = app.clone();
    let label2 = label.to_string();
    w.on_window_event(move |ev| {
        if let WindowEvent::Moved(p) = ev {
            on_badge_moved(&app2, &label2, *p);
        }
    });
    Some(w)
}

fn on_badge_moved(app: &AppHandle, label: &str, pos: PhysicalPosition<i32>) {
    let state = app.state::<AppState>();
    let seq = {
        let mut inner = state.lock();
        if inner.config.settings.badge.position != PositionMode::Free {
            return;
        }
        if inner
            .badge_geometry
            .get(label)
            .is_some_and(|g| g.x == pos.x && g.y == pos.y)
        {
            return; // our own move
        }
        let s = inner.move_seq.entry(label.to_string()).or_default();
        *s += 1;
        *s
    };
    let app = app.clone();
    let label = label.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(450)).await;
        let state = app.state::<AppState>();
        if state.lock().move_seq.get(&label) != Some(&seq) {
            return; // still dragging
        }
        finish_move(&app, &label);
    });
}

/// After a drag ends: snap to a nearby screen edge and remember the spot.
fn finish_move(app: &AppHandle, label: &str) {
    let Some(w) = app.get_webview_window(label) else {
        return;
    };
    let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) else {
        return;
    };
    let Some(mon) = w.current_monitor().ok().flatten() else {
        return;
    };
    let scale = mon.scale_factor();
    let wa = mon.work_area();
    let snap = (24.0 * scale) as i32;
    let m = (8.0 * scale).round() as i32;
    let (left, top) = (wa.position.x, wa.position.y);
    let (right, bottom) = (left + wa.size.width as i32, top + wa.size.height as i32);
    let (w_, h_) = (size.width as i32, size.height as i32);
    // Keep the whole badge on screen, then snap to a nearby edge.
    let mut x = pos.x.clamp(left, (right - w_).max(left));
    let mut y = pos.y.clamp(top, (bottom - h_).max(top));
    if (x - left).abs() < snap {
        x = left + m;
    } else if (right - (x + w_)).abs() < snap {
        x = right - w_ - m;
    }
    if (y - top).abs() < snap {
        y = top + m;
    } else if (bottom - (y + h_)).abs() < snap {
        y = bottom - h_ - m;
    }
    let state = app.state::<AppState>();
    {
        let mut inner = state.lock();
        inner.badge_geometry.insert(
            label.to_string(),
            Geometry {
                w: size.width,
                h: size.height,
                x,
                y,
            },
        );
        inner
            .config
            .badge_positions
            .insert(label.to_string(), SavedPos { x, y });
        state.save(&inner);
    }
    if (x, y) != (pos.x, pos.y) {
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_and_edge_placement_stay_on_screen() {
        let wa = Area {
            x: 0,
            y: 0,
            w: 1920,
            h: 1040,
        };
        let mut b = BadgeSettings::default();
        for pos in [PositionMode::Edge, PositionMode::Corner] {
            b.position = pos;
            for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
                b.edge = edge;
                for corner in [
                    Corner::TopLeft,
                    Corner::TopRight,
                    Corner::BottomLeft,
                    Corner::BottomRight,
                ] {
                    b.corner = corner;
                    for i in 0..3 {
                        let (x, y) = placed(&b, &wa, i, 3, 84, 84, 8);
                        assert!(
                            x >= 0 && y >= 0 && x + 84 <= 1920 && y + 84 <= 1040,
                            "{pos:?} {edge:?} {corner:?} {i}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn sizes_grow_with_people() {
        let (_, h1) = badge_logical_size(BadgeStyle::Ring, BadgeSize::M, 1);
        let (_, h2) = badge_logical_size(BadgeStyle::Ring, BadgeSize::M, 2);
        assert!(h2 > h1);
    }
}
