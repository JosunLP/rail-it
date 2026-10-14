//! Statischer Spurplan als SVG (Dokumentation, Projektierung).

use std::fmt::Write as _;

use crate::{LANE, Layout, Pos};

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn line(out: &mut String, a: Pos, b: Pos, class: &str) {
    let _ = writeln!(
        out,
        r#"<line class="{class}" x1="{}" y1="{}" x2="{}" y2="{}"/>"#,
        a[0], a[1], b[0], b[1]
    );
}

fn text(out: &mut String, at: Pos, class: &str, content: &str) {
    let _ = writeln!(
        out,
        r#"<text class="{class}" x="{}" y="{}">{}</text>"#,
        at[0],
        at[1],
        esc(content)
    );
}

/// Spurplan als eigenständiges SVG-Dokument.
pub fn to_svg(layout: &Layout) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" font-family="sans-serif" font-size="12">"#,
        w = layout.width,
        h = layout.height
    );
    out.push_str(
        "<style>.track{stroke:#444;stroke-width:4;stroke-linecap:round}\
         .border{stroke:#999;stroke-dasharray:4 4}\
         .area{fill:#888;font-size:11px}.name{fill:#333}.station{font-weight:bold;fill:#036}\
         .sig{fill:#c00}.lx{stroke:#a60;stroke-width:3}.crossing{stroke:#222;stroke-width:2}.crossing-link{stroke:#222;stroke-width:1;stroke-dasharray:4 4;opacity:.5}</style>\n",
    );
    let _ = writeln!(out, r##"<rect width="100%" height="100%" fill="#fff"/>"##);
    for a in &layout.areas {
        text(&mut out, [a.x0, 16], "area", &a.name);
    }
    for b in &layout.borders {
        line(&mut out, [b.at[0], 0], [b.at[0], layout.height], "border");
    }
    for s in &layout.stations {
        text(&mut out, s.at, "station", &s.name);
    }
    for t in &layout.tracks {
        for w in t.line.windows(2) {
            if let [a, b] = w {
                line(&mut out, *a, *b, "track");
            }
        }
        if let (Some(a), Some(b)) = (t.line.first(), t.line.last()) {
            text(
                &mut out,
                [i32::midpoint(a[0], b[0]), b[1] + 18],
                "name",
                &t.name,
            );
        }
    }
    for p in &layout.points {
        line(&mut out, p.tip, p.fork, "track");
        line(&mut out, p.fork, p.left, "track");
        line(&mut out, p.fork, p.right, "track");
        text(&mut out, [p.fork[0] - 8, p.fork[1] - 10], "name", &p.name);
    }
    for t in &layout.terminals {
        let [x, y] = t.at;
        match t.kind {
            "buffer_stop" => line(&mut out, [x, y - 10], [x, y + 10], "track"),
            _ => text(
                &mut out,
                [if t.opens == "right" { x - 24 } else { x + 6 }, y + 4],
                "name",
                &t.name,
            ),
        }
    }
    for s in &layout.signals {
        let [x, y] = s.at;
        // Signal neben dem Gleis, Dreieck in Wirkrichtung.
        let (dy, dir) = if s.facing == "right" {
            (LANE / 4, 1)
        } else {
            (-LANE / 4, -1)
        };
        let cy = y + dy;
        let _ = writeln!(
            out,
            r#"<polygon class="sig" points="{},{} {},{} {},{}"/>"#,
            x,
            cy - 6,
            x,
            cy + 6,
            x + 10 * dir,
            cy
        );
        text(
            &mut out,
            [x - 4, cy + if dir > 0 { 18 } else { -10 }],
            "name",
            &s.name,
        );
    }
    for c in &layout.crossings {
        let [a, b] = c.at;
        line(&mut out, a, b, "crossing-link");
        for [x, y] in [a, b] {
            line(&mut out, [x - 7, y - 7], [x + 7, y + 7], "crossing");
            line(&mut out, [x - 7, y + 7], [x + 7, y - 7], "crossing");
        }
        text(&mut out, [a[0] - 34, a[1] - 10], "name", &c.name);
    }
    for l in &layout.level_crossings {
        let [x, y] = l.at;
        line(&mut out, [x - 6, y - 14], [x - 6, y + 14], "lx");
        line(&mut out, [x + 6, y - 14], [x + 6, y + 14], "lx");
        text(&mut out, [x + 10, y - 16], "name", &l.name);
    }
    out.push_str("</svg>\n");
    out
}
