use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};
use egui::epaint::PathShape;

/// Draw the icon portion of a NATO military symbol SVG into `target_rect`.
///
/// The frame (rectangle / diamond / square / circle) is drawn separately by
/// `render::draw_frame`; this function only renders the interior function icon.
/// Paths whose `id` ends with `"Fill"` or `"Frame"` are considered infrastructure
/// and are skipped. All path colors in the SVG are replaced by `color`.
pub fn draw_svg_icon(painter: &Painter, svg_bytes: &[u8], target_rect: Rect, color: Color32) {
    let text = match std::str::from_utf8(svg_bytes) {
        Ok(t) => t,
        Err(_) => return,
    };
    let doc = match roxmltree::Document::parse(text) {
        Ok(d) => d,
        Err(_) => return,
    };

    let root = doc.root_element();
    let vw = attr_f32(root, "width", 600.0);
    let vh = attr_f32(root, "height", 400.0);

    // Uniform scale to fit target_rect, centered.
    let scale = (target_rect.width() / vw).min(target_rect.height() / vh);
    let ox = target_rect.left() + (target_rect.width() - vw * scale) * 0.5;
    let oy = target_rect.top()  + (target_rect.height() - vh * scale) * 0.5;
    let to_pos = |x: f32, y: f32| Pos2::new(ox + x * scale, oy + y * scale);

    // Stroke width: scale the typical SVG stroke (5–6px in a 600-wide viewport)
    // and clamp so it stays visible at small sizes.
    let sw = (scale * 6.0).clamp(1.0, 3.0);

    for node in doc.descendants() {
        if node.tag_name().name() != "path" { continue; }

        let id = node.attribute("id").unwrap_or("");
        if is_infrastructure(id) { continue; }

        let d = match node.attribute("d") {
            Some(d) => d,
            None    => continue,
        };

        let style = node.attribute("style").unwrap_or("");
        let fill_none = style.contains("fill:none");

        for (closed, raw) in parse_path(d) {
            if raw.len() < 2 { continue; }
            let pts: Vec<Pos2> = raw.iter().map(|[x, y]| to_pos(*x, *y)).collect();

            if !fill_none && closed && pts.len() >= 3 {
                painter.add(Shape::Path(PathShape {
                    points: pts,
                    closed: true,
                    fill:   color,
                    stroke: egui::epaint::PathStroke::NONE,
                }));
            } else {
                let mut draw = pts;
                if closed { draw.push(draw[0]); }
                painter.add(Shape::line(draw, Stroke::new(sw, color)));
            }
        }
    }
}

// ─── SVG path parser ─────────────────────────────────────────────────────────

/// Parse an SVG `d` attribute into a list of `(closed, points)` subpaths.
/// Cubic and quadratic bezier curves are tessellated to straight line segments.
fn parse_path(d: &str) -> Vec<(bool, Vec<[f32; 2]>)> {
    let mut subpaths: Vec<(bool, Vec<[f32; 2]>)> = Vec::new();
    let mut pts: Vec<[f32; 2]> = Vec::new();
    let mut cx = 0.0_f32;
    let mut cy = 0.0_f32;
    let mut sx = 0.0_f32;
    let mut sy = 0.0_f32;
    let mut cmd = ' ';

    let tokens = tokenize(d);
    let mut i = 0;

    macro_rules! num {
        ($j:expr) => {
            match tokens.get($j) {
                Some(Tok::N(n)) => *n,
                _               => return subpaths,
            }
        };
    }

    while i < tokens.len() {
        match tokens[i] {
            Tok::C(c) => {
                cmd = c;
                i += 1;
                if c == 'Z' || c == 'z' {
                    if !pts.is_empty() {
                        subpaths.push((true, std::mem::take(&mut pts)));
                    }
                    cx = sx; cy = sy;
                }
            }
            Tok::N(_) => match cmd {
                'M' | 'm' => {
                    if i + 1 < tokens.len() {
                        let (x, y) = (num!(i), num!(i + 1));
                        if cmd == 'm' { cx += x; cy += y; } else { cx = x; cy = y; }
                        sx = cx; sy = cy;
                        if !pts.is_empty() {
                            subpaths.push((false, std::mem::take(&mut pts)));
                        }
                        pts.push([cx, cy]);
                        i += 2;
                        cmd = if cmd == 'm' { 'l' } else { 'L' };
                    } else { break; }
                }
                'L' => { cx = num!(i); cy = num!(i+1); pts.push([cx, cy]); i += 2; }
                'l' => { cx += num!(i); cy += num!(i+1); pts.push([cx, cy]); i += 2; }
                'H' => { cx = num!(i); pts.push([cx, cy]); i += 1; }
                'h' => { cx += num!(i); pts.push([cx, cy]); i += 1; }
                'V' => { cy = num!(i); pts.push([cx, cy]); i += 1; }
                'v' => { cy += num!(i); pts.push([cx, cy]); i += 1; }
                'C' => {
                    if i + 5 < tokens.len() {
                        let (p1, p2) = ([num!(i), num!(i+1)], [num!(i+2), num!(i+3)]);
                        let p3 = [num!(i+4), num!(i+5)];
                        cubic([cx, cy], p1, p2, p3, &mut pts);
                        cx = p3[0]; cy = p3[1]; i += 6;
                    } else { break; }
                }
                'c' => {
                    if i + 5 < tokens.len() {
                        let p1 = [cx + num!(i), cy + num!(i+1)];
                        let p2 = [cx + num!(i+2), cy + num!(i+3)];
                        let p3 = [cx + num!(i+4), cy + num!(i+5)];
                        cubic([cx, cy], p1, p2, p3, &mut pts);
                        cx = p3[0]; cy = p3[1]; i += 6;
                    } else { break; }
                }
                'Q' => {
                    if i + 3 < tokens.len() {
                        let p1 = [num!(i), num!(i+1)];
                        let p2 = [num!(i+2), num!(i+3)];
                        quad([cx, cy], p1, p2, &mut pts);
                        cx = p2[0]; cy = p2[1]; i += 4;
                    } else { break; }
                }
                'q' => {
                    if i + 3 < tokens.len() {
                        let p1 = [cx + num!(i), cy + num!(i+1)];
                        let p2 = [cx + num!(i+2), cy + num!(i+3)];
                        quad([cx, cy], p1, p2, &mut pts);
                        cx = p2[0]; cy = p2[1]; i += 4;
                    } else { break; }
                }
                _ => { i += 1; }
            },
        }
    }

    if !pts.is_empty() {
        subpaths.push((false, pts));
    }
    subpaths
}

#[derive(Clone, Copy)]
enum Tok { C(char), N(f32) }

/// Tokenize an SVG path `d` attribute into commands and numbers.
/// Handles implicit sign-separated numbers (e.g. `5-3` → [5, -3]).
fn tokenize(d: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let b = d.as_bytes();
    let mut i = 0;

    while i < b.len() {
        let c = b[i] as char;

        if c.is_ascii_whitespace() || c == ',' { i += 1; continue; }

        if c.is_ascii_alphabetic() {
            out.push(Tok::C(c));
            i += 1;
            continue;
        }

        if c.is_ascii_digit() || c == '.' || c == '-' {
            let start = i;
            let mut has_dot = c == '.';
            let mut has_exp = false;
            i += 1;
            while i < b.len() {
                match b[i] as char {
                    d if d.is_ascii_digit() => { i += 1; }
                    '.' if !has_dot && !has_exp => { has_dot = true; i += 1; }
                    'e' | 'E' if !has_exp => {
                        has_exp = true; i += 1;
                        if i < b.len() && matches!(b[i] as char, '+' | '-') { i += 1; }
                    }
                    // A '-' or '.' after digits starts a new number — stop here.
                    '-' | '.' => break,
                    _ => break,
                }
            }
            if let Ok(n) = d[start..i].parse::<f32>() {
                out.push(Tok::N(n));
            }
            continue;
        }

        i += 1;
    }
    out
}

// ─── Bezier tessellation ─────────────────────────────────────────────────────

const STEPS: usize = 12;

fn cubic(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], out: &mut Vec<[f32; 2]>) {
    for k in 1..=STEPS {
        let t = k as f32 / STEPS as f32;
        let u = 1.0 - t;
        out.push([
            u*u*u*p0[0] + 3.0*u*u*t*p1[0] + 3.0*u*t*t*p2[0] + t*t*t*p3[0],
            u*u*u*p0[1] + 3.0*u*u*t*p1[1] + 3.0*u*t*t*p2[1] + t*t*t*p3[1],
        ]);
    }
}

fn quad(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], out: &mut Vec<[f32; 2]>) {
    for k in 1..=STEPS {
        let t = k as f32 / STEPS as f32;
        let u = 1.0 - t;
        out.push([
            u*u*p0[0] + 2.0*u*t*p1[0] + t*t*p2[0],
            u*u*p0[1] + 2.0*u*t*p1[1] + t*t*p2[1],
        ]);
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn is_infrastructure(id: &str) -> bool {
    id.ends_with("Fill") || id.ends_with("Frame")
}

fn attr_f32(node: roxmltree::Node, attr: &str, default: f32) -> f32 {
    node.attribute(attr).and_then(|s| s.parse().ok()).unwrap_or(default)
}
