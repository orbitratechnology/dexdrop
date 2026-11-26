//! Pictures of a headless screen. NSH's headless engine lays the app out
//! but draws nothing (`screenshot()` needs a real platform); this draws
//! what it laid out: each view's frame with the style the app mounted
//! (background, corner radius, border, opacity), its image (the app's SVG
//! icons tinted as mounted, picture files), and its text (font size, weight,
//! color, alignment), clipped as the views around it clip. The result is an
//! SVG rasterized with resvg at 2×.
//!
//! What it leaves out: shadows, blur and glass, transforms, the platform's
//! own controls' drawing (a native switch or segmented control shows as its
//! frame), and exact text shaping (the headless engine's text sizes are
//! approximate). It shows layout, color and content, not pixels.

use std::{
    collections::HashMap,
    fmt::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use nsh_core::{
    HeaderTitleAlign, MountItem, NodeId, ResizeMode, ScreenConfig, TextAlign, ViewKind, ViewStyle,
};
use nsh_testing::{Headless, LayoutNode};

/// What the app mounted, by view: its style and its image.
#[derive(Default)]
struct Mounted {
    styles: HashMap<NodeId, ViewStyle>,
    screens: HashMap<NodeId, ScreenConfig>,
    images: HashMap<NodeId, (String, ResizeMode)>,
}

/// Records what the app mounts, from the first batch on: call it right
/// after `Headless::start`, before any step.
pub struct Camera {
    mounted: Arc<Mutex<Mounted>>,
}

impl Camera {
    pub fn attach(app: &mut Headless) -> Camera {
        let mounted = Arc::new(Mutex::new(Mounted::default()));
        let log = mounted.clone();
        app.observe(move |_step, batch| {
            let mut m = log.lock().unwrap();
            for item in &batch.items {
                match item {
                    MountItem::ViewStyle { id, style } => {
                        m.styles.insert(*id, (**style).clone());
                    }
                    MountItem::ScreenConfig { id, config } => {
                        m.screens.insert(*id, (**config).clone());
                    }
                    MountItem::Image {
                        id,
                        source,
                        resize_mode,
                        ..
                    } => match source {
                        Some(s) => {
                            m.images.insert(*id, (s.uri.to_string(), *resize_mode));
                        }
                        None => {
                            m.images.remove(id);
                        }
                    },
                    _ => {}
                }
            }
        });
        Camera { mounted }
    }

    /// Writes the screen as `<dir>/<name>.png` (and `.svg`), drawn at 2×.
    pub fn shoot(&self, app: &mut Headless, dir: &Path, name: &str) -> PathBuf {
        app.settle();
        std::fs::create_dir_all(dir).unwrap();
        let nodes = app.screen().layout_nodes();
        let bg = |id: NodeId| {
            app.screen()
                .view_style(&format!("view:{id}"))
                .ok()
                .flatten()
                .and_then(|s| s.background_color)
        };
        let backgrounds: HashMap<NodeId, u32> = nodes
            .iter()
            .filter_map(|n| bg(n.id).map(|c| (n.id, c)))
            .collect();
        let svg = self.svg(&nodes, &backgrounds, dir);
        std::fs::write(dir.join(format!("{name}.svg")), &svg).unwrap();
        let png = dir.join(format!("{name}.png"));
        rasterize(&svg, dir, &png);
        png
    }

    fn svg(&self, nodes: &[LayoutNode], backgrounds: &HashMap<NodeId, u32>, dir: &Path) -> String {
        let m = self.mounted.lock().unwrap();
        let root = nodes
            .iter()
            .map(|n| n.bounds)
            .fold((0f32, 0f32), |(w, h), b| {
                (
                    w.max(b.origin.x.0 + b.size.width.0),
                    h.max(b.origin.y.0 + b.size.height.0),
                )
            });
        // The window: the first full-size node's size.
        let (w, h) = nodes
            .iter()
            .find(|n| n.bounds.size.width.0 > 0.0 && n.bounds.size.height.0 > 0.0)
            .map(|n| (n.bounds.size.width.0, n.bounds.size.height.0))
            .unwrap_or(root);
        let by_id: HashMap<NodeId, &LayoutNode> = nodes.iter().map(|n| (n.id, n)).collect();
        let mut defs = String::new();
        let mut body = String::new();
        let page = nodes
            .iter()
            .find_map(|n| backgrounds.get(&n.id).copied())
            .unwrap_or(0xffff_ffff);
        writeln!(
            body,
            r#"<rect width="{w}" height="{h}" fill="{}"/>"#,
            color(page)
        )
        .unwrap();
        let mut clip_ids = HashMap::new();
        let mut opacity_of: HashMap<NodeId, f32> = HashMap::new();
        // A phone's header is the system's bar, drawn below from the
        // screen's config: the app's own header views (desktops') are left
        // out. Modals go over the page.
        let under = |n: &LayoutNode, kind: ViewKind| {
            let mut at = Some(n);
            while let Some(v) = at {
                if v.kind == kind {
                    return true;
                }
                at = by_id.get(&v.parent).copied().filter(|p| p.id != v.id);
            }
            false
        };
        let mut ordered: Vec<&LayoutNode> = nodes
            .iter()
            .filter(|n| !under(n, ViewKind::HeaderItem))
            .collect();
        ordered.sort_by_key(|n| under(n, ViewKind::Modal));
        for n in ordered {
            if n.hidden {
                continue;
            }
            let style = m.styles.get(&n.id);
            let own = style.map_or(1.0, |s| s.opacity);
            let opacity = by_id
                .get(&n.parent)
                .and_then(|p| opacity_of.get(&p.id))
                .copied()
                .unwrap_or(1.0)
                * own;
            opacity_of.insert(n.id, opacity);
            if opacity <= 0.01 {
                continue;
            }
            let b = n.bounds;
            let (x, y, bw, bh) = (b.origin.x.0, b.origin.y.0, b.size.width.0, b.size.height.0);
            if bw <= 0.0 || bh <= 0.0 || y > h || x > w || y + bh < 0.0 || x + bw < 0.0 {
                continue;
            }
            let radius = style.map_or(0.0, |s| s.border_radius.0.min(bw / 2.0).min(bh / 2.0));
            // The clipping views around it, outermost first.
            let mut chain = Vec::new();
            let mut p = n.parent;
            while let Some(parent) = by_id.get(&p) {
                if parent.clips || parent.scroll.is_some() {
                    chain.push(parent.id);
                }
                p = parent.parent;
            }
            chain.reverse();
            let mut open = 0;
            for c in &chain {
                let cid = clip_ids.len();
                let key = *clip_ids.entry(*c).or_insert_with(|| {
                    let cb = by_id[c].bounds;
                    let r = m.styles.get(c).map_or(0.0, |s| s.border_radius.0);
                    writeln!(
                        defs,
                        r#"<clipPath id="k{cid}"><rect x="{}" y="{}" width="{}" height="{}" rx="{r}"/></clipPath>"#,
                        cb.origin.x.0, cb.origin.y.0, cb.size.width.0, cb.size.height.0
                    )
                    .unwrap();
                    cid
                });
                write!(body, r#"<g clip-path="url(#k{key})">"#).unwrap();
                open += 1;
            }
            write!(body, r#"<g opacity="{own}">"#).unwrap();
            open += 1;
            if let Some(c) = backgrounds.get(&n.id).filter(|c| *c & 0xff != 0) {
                writeln!(
                    body,
                    r#"<rect x="{x}" y="{y}" width="{bw}" height="{bh}" rx="{radius}" fill="{}"/>"#,
                    color(*c)
                )
                .unwrap();
            }
            if let Some(s) = style {
                let bwid = s.borders.widths.top.0;
                let bcol = s.borders.colors[0].0;
                if bwid > 0.0 && bcol & 0xff != 0 {
                    let i = bwid / 2.0;
                    writeln!(
                        body,
                        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="none" stroke="{}" stroke-width="{bwid}"/>"#,
                        x + i,
                        y + i,
                        bw - bwid,
                        bh - bwid,
                        (radius - i).max(0.0),
                        color(bcol)
                    )
                    .unwrap();
                }
            }
            if let Some((uri, mode)) = m.images.get(&n.id)
                && let Some(file) =
                    image_file(uri, style.and_then(|s| s.tint_color.map(|c| c.0)), dir)
            {
                let fit = match mode {
                    ResizeMode::Cover => "xMidYMid slice",
                    ResizeMode::Stretch => "none",
                    _ => "xMidYMid meet",
                };
                writeln!(
                    body,
                    r#"<image x="{x}" y="{y}" width="{bw}" height="{bh}" preserveAspectRatio="{fit}" href="{}"/>"#,
                    xml(&file.to_string_lossy())
                )
                .unwrap();
            }
            if n.kind == ViewKind::Text
                && let Some(t) = &n.text
            {
                text(&mut body, t);
            }
            for _ in 0..open {
                body.push_str("</g>");
            }
            body.push('\n');
        }
        // The system's bar of the screen showing: its title, a back arrow
        // on a pushed screen, over the room the page leaves above it.
        if let Some((screen, config)) = nodes
            .iter()
            .rev()
            .filter(|n| n.kind == ViewKind::Screen && !n.hidden)
            .filter_map(|n| m.screens.get(&n.id).map(|c| (n, c)))
            .find(|(_, c)| c.header_visible)
        {
            let top = screen.bounds.origin.y.0;
            let content = nodes
                .iter()
                .filter(|n| n.parent == screen.id && !under(n, ViewKind::HeaderItem))
                .map(|n| n.bounds.origin.y.0)
                .fold(f32::MAX, f32::min);
            let bar = if content > top && content < top + 120.0 {
                content - top
            } else {
                56.0
            };
            let fill = config.background_color.map_or(page, |c| c.0);
            let ink = config.title_color.map_or(0x0d0d_0dff, |c| c.0);
            let tint = config.tint_color.map_or(ink, |c| c.0);
            writeln!(
                body,
                r#"<rect x="0" y="{top}" width="{w}" height="{bar}" fill="{}"/>"#,
                color(fill)
            )
            .unwrap();
            let mid = top + bar / 2.0;
            let mut x = 16.0;
            if config.back_visible {
                writeln!(
                    body,
                    r#"<path d="M{} {mid} h-14 m6 -6 l-6 6 l6 6" fill="none" stroke="{}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#,
                    x + 17.0,
                    color(tint)
                )
                .unwrap();
                x = 56.0;
            }
            if let Some(title) = &config.title {
                let (anchor, tx) = match config.title_align {
                    HeaderTitleAlign::Center => ("middle", w / 2.0),
                    _ => ("start", x),
                };
                writeln!(
                    body,
                    r#"<text x="{tx}" y="{}" font-family="Inter, DejaVu Sans, sans-serif" font-size="20" font-weight="600" fill="{}" text-anchor="{anchor}">{}</text>"#,
                    mid + 7.0,
                    color(ink),
                    xml(title)
                )
                .unwrap();
            }
        }
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><defs>{defs}</defs>{body}</svg>"#
        )
    }
}

/// A text view's lines: wrapped at its box's width (by an estimate of the
/// characters' widths), cut at its line limit with an ellipsis.
fn text(out: &mut String, t: &nsh_testing::LayoutText) {
    let a = &t.text;
    let Some(span) = a.spans.first() else { return };
    let s = &span.style;
    let size = s.font_size.0.max(1.0);
    let line = if s.line_height.0 > 0.0 {
        s.line_height.0
    } else {
        size * 1.25
    };
    let c = t.content;
    let (x0, y0, width) = (c.origin.x.0, c.origin.y.0, c.size.width.0.max(1.0));
    // As many lines as layout made room for: the box's height in lines.
    let rows = ((c.size.height.0 / line).round() as usize).max(1);
    let total = a.text.chars().count();
    let per_char = size * if s.font_weight >= 600 { 0.56 } else { 0.52 };
    let fits = if rows == 1 {
        usize::MAX
    } else {
        ((width / per_char).floor() as usize).max(total.div_ceil(rows))
    };
    let mut lines: Vec<String> = Vec::new();
    for para in a.text.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            let next = if cur.is_empty() {
                word.to_owned()
            } else {
                format!("{cur} {word}")
            };
            if next.chars().count() > fits && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_owned();
            } else {
                cur = next;
            }
        }
        lines.push(cur);
    }
    let max = if a.max_lines > 0 {
        (a.max_lines as usize).min(rows)
    } else {
        rows
    };
    if lines.len() > max {
        let rest = lines.split_off(max);
        if let Some(last) = lines.last_mut() {
            last.push(' ');
            last.push_str(&rest.join(" "));
        }
    }
    let (anchor, ax) = match a.align {
        TextAlign::Center => ("middle", x0 + width / 2.0),
        TextAlign::Right => ("end", x0 + width),
        _ => ("start", x0),
    };
    // Box taller than its lines: centered in it, as a single line is.
    let used = line * lines.len() as f32;
    let top = y0 + (c.size.height.0 - used) / 2.0;
    for (i, l) in lines.iter().enumerate() {
        let l = l.clone();
        let baseline = top + line * i as f32 + (line + size * 0.7) / 2.0;
        writeln!(
            out,
            r#"<text x="{ax}" y="{baseline}" font-family="Inter, DejaVu Sans, sans-serif" font-size="{size}" font-weight="{}" fill="{}" text-anchor="{anchor}">{}</text>"#,
            s.font_weight,
            color(s.color.0),
            xml(&l)
        )
        .unwrap();
    }
}

/// The file an image source names: one of the app's SVG assets (tinted
/// with `tint` into a copy in `dir`), or a picture file.
fn image_file(uri: &str, tint: Option<u32>, dir: &Path) -> Option<PathBuf> {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let path = if let Some(i) = uri.find("icons/").or_else(|| uri.find("art/")) {
        assets.join(&uri[i..])
    } else {
        PathBuf::from(uri.trim_start_matches("file://"))
    };
    if !path.is_file() {
        return None;
    }
    let Some(tint) = tint.filter(|_| path.extension().is_some_and(|e| e == "svg")) else {
        return Some(path);
    };
    let svg = std::fs::read_to_string(&path).ok()?;
    let c = color(tint);
    let tinted = svg
        .replace("#000000", &c)
        .replace("#000", &c)
        .replace("currentColor", &c)
        .replace("black", &c);
    let to = dir.join("tinted").join(format!(
        "{}-{tint:08x}.svg",
        path.file_stem()?.to_string_lossy()
    ));
    std::fs::create_dir_all(to.parent()?).ok()?;
    std::fs::write(&to, tinted).ok()?;
    Some(to)
}

/// `0xRRGGBBAA` as an SVG color.
fn color(c: u32) -> String {
    let (r, g, b, a) = (c >> 24, (c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff);
    if a == 0xff {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("rgba({r},{g},{b},{:.3})", a as f32 / 255.0)
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Draws `svg` into `png` at 2×, with the system's fonts.
fn rasterize(svg: &str, dir: &Path, png: &Path) {
    let mut opt = resvg::usvg::Options {
        resources_dir: Some(dir.to_path_buf()),
        ..Default::default()
    };
    opt.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(svg, &opt).expect("the screen's SVG");
    let size = tree.size().to_int_size().scale_by(2.0).unwrap();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(2.0, 2.0),
        &mut pixmap.as_mut(),
    );
    pixmap.save_png(png).unwrap();
}
