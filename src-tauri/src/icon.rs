//! Draws tray icons at runtime: a colored square with the reading in a pixel font,
//! or a colored dot. No font files or image libraries needed.

use crate::model::Status;

/// What a tray icon should show.
pub struct IconSpec<'a> {
    pub status: Status,
    /// Up to 3 characters, e.g. "142", "7.9", "LO". None shows a placeholder.
    pub text: Option<&'a str>,
    /// +2..-2, drawn as a notch on the top (up) or bottom (down) edge.
    pub direction: i8,
    /// Small white corner mark so demo data is never mistaken for real data.
    pub demo: bool,
}

struct Canvas {
    size: u32,
    px: Vec<[f32; 4]>, // straight RGBA, 0..1
}

impl Canvas {
    fn new(size: u32) -> Self {
        Canvas {
            size,
            px: vec![[0.0; 4]; (size * size) as usize],
        }
    }

    /// Fills every pixel covered by `inside`, with 4×4 supersampling for smooth edges.
    fn fill(&mut self, rgb: [u8; 3], inside: impl Fn(f32, f32) -> bool) {
        let c = [
            rgb[0] as f32 / 255.0,
            rgb[1] as f32 / 255.0,
            rgb[2] as f32 / 255.0,
        ];
        for y in 0..self.size {
            for x in 0..self.size {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        if inside(
                            x as f32 + (sx as f32 + 0.5) / 4.0,
                            y as f32 + (sy as f32 + 0.5) / 4.0,
                        ) {
                            hits += 1;
                        }
                    }
                }
                if hits == 0 {
                    continue;
                }
                let a = hits as f32 / 16.0;
                let p = &mut self.px[(y * self.size + x) as usize];
                let out_a = a + p[3] * (1.0 - a);
                for i in 0..3 {
                    p[i] = (c[i] * a + p[i] * p[3] * (1.0 - a)) / out_a;
                }
                p[3] = out_a;
            }
        }
    }

    fn rgba(&self) -> Vec<u8> {
        self.px
            .iter()
            .flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect()
    }
}

/// 3×5 pixel glyphs. Each row is 3 bits, left to right.
fn glyph(c: char) -> Option<(u8, [u8; 5])> {
    let g = |rows: [u8; 5]| Some((3u8, rows));
    match c {
        '0' | 'O' => g([0b111, 0b101, 0b101, 0b101, 0b111]),
        '1' => g([0b010, 0b110, 0b010, 0b010, 0b111]),
        '2' => g([0b111, 0b001, 0b111, 0b100, 0b111]),
        '3' => g([0b111, 0b001, 0b111, 0b001, 0b111]),
        '4' => g([0b101, 0b101, 0b111, 0b001, 0b001]),
        '5' => g([0b111, 0b100, 0b111, 0b001, 0b111]),
        '6' => g([0b111, 0b100, 0b111, 0b101, 0b111]),
        '7' => g([0b111, 0b001, 0b001, 0b001, 0b001]),
        '8' => g([0b111, 0b101, 0b111, 0b101, 0b111]),
        '9' => g([0b111, 0b101, 0b111, 0b001, 0b111]),
        '-' => g([0b000, 0b000, 0b111, 0b000, 0b000]),
        'L' => g([0b100, 0b100, 0b100, 0b100, 0b111]),
        'H' => g([0b101, 0b101, 0b111, 0b101, 0b101]),
        'I' => g([0b111, 0b010, 0b010, 0b010, 0b111]),
        '.' => Some((1, [0, 0, 0, 0, 1])),
        '!' => Some((1, [1, 1, 1, 0, 1])),
        _ => None,
    }
}

fn rounded_rect(x: f32, y: f32, s: f32, r: f32) -> bool {
    if x < 0.0 || y < 0.0 || x > s || y > s {
        return false;
    }
    let cx = x.clamp(r, s - r);
    let cy = y.clamp(r, s - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

fn triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let sign = |p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)| {
        (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
    };
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

const WHITE: [u8; 3] = [255, 255, 255];

/// Draws `text` centered in the box `inset` px from the left/right edges, from `top` down `avail_h` px.
fn draw_text(cv: &mut Canvas, text: &str, inset: f32, top: f32, avail_h: f32) {
    let s = cv.size as f32;
    let glyphs: Vec<(u8, [u8; 5])> = text.chars().filter_map(glyph).collect();
    if glyphs.is_empty() {
        return;
    }
    let cells: u32 = glyphs.iter().map(|g| g.0 as u32).sum::<u32>() + glyphs.len() as u32 - 1;
    // Whole-pixel cell sizes keep the digits crisp at 16 px. Cells are never wider than
    // tall, so a short text like "!" doesn't stretch into a block.
    let sy_max = (avail_h / 5.0).floor().max(1.0);
    let sx = ((s - 2.0 * inset) / cells as f32)
        .floor()
        .max(1.0)
        .min(sy_max);
    let sy = sy_max.min(sx * 2.0);
    let width = cells as f32 * sx;
    let mut x0 = ((s - width) / 2.0).round();
    let y0 = (top + (avail_h - 5.0 * sy) / 2.0).round();
    for (w, rows) in glyphs {
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..w {
                if bits >> (w - 1 - col) & 1 == 1 {
                    let (gx, gy) = (x0 + col as f32 * sx, y0 + row as f32 * sy);
                    cv.fill(WHITE, |x, y| {
                        x >= gx && x < gx + sx && y >= gy && y < gy + sy
                    });
                }
            }
        }
        x0 += (w as f32 + 1.0) * sx;
    }
}

/// Square icon with the number. Returns RGBA bytes, `size`×`size`.
pub fn number_icon(spec: &IconSpec, size: u32) -> Vec<u8> {
    let mut cv = Canvas::new(size);
    let s = size as f32;
    let radius = (s * 0.2).max(2.0);
    cv.fill(spec.status.rgb(), |x, y| rounded_rect(x, y, s, radius));

    let notch = (s / 8.0).round().max(2.0);
    let text = match (spec.status, spec.text) {
        (Status::SignIn, _) => "!",
        (st, Some(t)) if st.shows_value() => t,
        _ => "--",
    };
    if spec.status.shows_value() && spec.direction != 0 {
        let half = notch * if spec.direction.abs() == 2 { 2.0 } else { 1.3 };
        let mid = s / 2.0;
        if spec.direction > 0 {
            cv.fill(WHITE, |x, y| {
                triangle(
                    (x, y),
                    (mid - half, notch + 0.5),
                    (mid + half, notch + 0.5),
                    (mid, 0.5),
                )
            });
        } else {
            cv.fill(WHITE, |x, y| {
                triangle(
                    (x, y),
                    (mid - half, s - notch - 0.5),
                    (mid + half, s - notch - 0.5),
                    (mid, s - 0.5),
                )
            });
        }
    }
    if spec.status == Status::UrgentLow {
        // Inner white ring marks urgent low without relying on color.
        let w = (s / 16.0).max(1.0);
        cv.fill(WHITE, |x, y| {
            rounded_rect(x, y, s, radius) && !rounded_rect(x - w, y - w, s - 2.0 * w, radius - w)
        });
    }
    // Keep digits clear of the urgent-low ring so they never merge with it.
    let inset = if spec.status == Status::UrgentLow {
        (s / 16.0).max(1.0) + (s / 16.0).max(1.0).ceil()
    } else {
        (s * 0.04).max(0.5)
    };
    draw_text(
        &mut cv,
        text,
        inset,
        notch.max(inset),
        s - 2.0 * notch.max(inset),
    );
    if spec.demo {
        let m = (s / 8.0).max(2.0);
        cv.fill([255, 209, 102], |x, y| x < m && y < m);
    }
    cv.rgba()
}

/// Round colored dot (macOS / Linux, where the number is shown as text beside it).
pub fn dot_icon(spec: &IconSpec, size: u32) -> Vec<u8> {
    let mut cv = Canvas::new(size);
    let s = size as f32;
    let r = s * 0.32;
    let c = s / 2.0;
    cv.fill(spec.status.rgb(), |x, y| {
        (x - c).powi(2) + (y - c).powi(2) <= r * r
    });
    if spec.status == Status::SignIn {
        draw_text(&mut cv, "!", 0.0, c - r, 2.0 * r);
    }
    if spec.demo {
        let m = (s / 6.0).max(2.0);
        cv.fill([255, 209, 102], |x, y| x < m && y < m);
    }
    cv.rgba()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(status: Status, text: &str, dir: i8) -> IconSpec<'_> {
        IconSpec {
            status,
            text: Some(text),
            direction: dir,
            demo: false,
        }
    }

    #[test]
    fn sizes_and_opaque_center() {
        for size in [16, 20, 24, 32, 64] {
            let px = number_icon(&spec(Status::InRange, "142", 1), size);
            assert_eq!(px.len(), (size * size * 4) as usize);
        }
    }

    #[test]
    fn digits_are_drawn_in_white() {
        let px = number_icon(&spec(Status::High, "245", 0), 32);
        let white = px
            .chunks(4)
            .filter(|p| p[0] > 240 && p[1] > 240 && p[2] > 240 && p[3] > 240)
            .count();
        assert!(
            white > 30,
            "expected visible digits, got {white} white pixels"
        );
    }

    #[test]
    fn every_icon_value_has_glyphs() {
        for t in ["142", "7.9", "10", "LO", "HI", "--", "!"] {
            assert!(t.chars().all(|c| glyph(c).is_some()), "{t}");
        }
    }

    #[test]
    fn dot_icon_has_transparent_corners() {
        let px = dot_icon(&spec(Status::Low, "", 0), 32);
        assert_eq!(px[3], 0);
    }
}

/// Writes sample icons as BMP files for eyeballing: `ICON_DUMP=/some/dir cargo test dump -- --ignored`
#[cfg(test)]
mod dump {
    use super::*;

    fn bmp(path: &std::path::Path, rgba: &[u8], size: u32) {
        let mut out = Vec::new();
        let data_len = size * size * 4;
        out.extend(b"BM");
        out.extend((54 + data_len).to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(54u32.to_le_bytes());
        out.extend(40u32.to_le_bytes());
        out.extend((size as i32).to_le_bytes());
        out.extend((-(size as i32)).to_le_bytes()); // top-down
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend([0u8; 24]);
        for p in rgba.chunks(4) {
            // Composite onto a dark taskbar grey so transparency is visible.
            let a = p[3] as u32;
            let bg = 32u32;
            let c = |v: u8| ((v as u32 * a + bg * (255 - a)) / 255) as u8;
            out.extend([c(p[2]), c(p[1]), c(p[0]), 255]);
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    #[ignore]
    fn dump() {
        let Ok(dir) = std::env::var("ICON_DUMP") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        let cases = [
            ("inrange", Status::InRange, "142", 0),
            ("high_up", Status::High, "245", 1),
            ("low_down", Status::Low, "68", -1),
            ("urgent", Status::UrgentLow, "52", -2),
            ("stale", Status::Stale, "142", 0),
            ("mmol", Status::InRange, "7.9", 1),
            ("signin", Status::SignIn, "", 0),
        ];
        for size in [16, 20, 24, 32] {
            for (name, status, text, dir_) in cases {
                let spec = IconSpec {
                    status,
                    text: Some(text),
                    direction: dir_,
                    demo: false,
                };
                bmp(
                    &dir.join(format!("{size}_{name}.bmp")),
                    &number_icon(&spec, size),
                    size,
                );
            }
        }
    }
}
