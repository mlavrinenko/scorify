//! Native output: a system's draw commands resolved into [`NativeCmd`]s for a
//! frontend that places content itself, instead of one SVG image.
//!
//! The renderer works in y-up mm with origin moves and anchored glyphs. Here
//! every item lands in the system's [`Viewport`] (top-left origin, y down, the
//! box the SVG form takes) with moves applied, and each music glyph becomes
//! music-font text at its baseline origin. A Typst frontend can then draw
//! glyphs as text, which its glyph cache rasterises once, and a change to one
//! notehead's colour stays a change to one item.

use crate::glyph;
use crate::renderer::{
    anchor_sw, anchored_text_origin, glyph_id, music_text_bbox, smufl_bbox_for_codepoint, Viewport,
    LELAND_FONT,
};
use crate::types::{DrawCmd, NativeCmd};
use ttf_parser::Face;

/// Resolve `cmds` into native items in `view`'s frame.
///
/// Glyphs land where the SVG form puts them: with the bundled Leland face, the
/// glyph's ink box is anchored exactly as the SVG anchors its outline; for any
/// other font, at the SMuFL-metadata origin the SVG's text fallback uses.
pub(crate) fn native_from_cmds(
    cmds: &[DrawCmd],
    view: &Viewport,
    music_font: &str,
) -> Vec<NativeCmd> {
    let font = glyph::FontId::from_name(music_font);
    let face = if music_font == "Leland" {
        Face::parse(LELAND_FONT, 0).ok()
    } else {
        None
    };
    let point = |x: f64, y: f64| (view.x(x), view.y(y));
    let points = |pts: &[f64], ox: f64, oy: f64| {
        pts.chunks_exact(2)
            .flat_map(|xy| {
                let (x, y) = point(ox + xy[0], oy + xy[1]);
                [x, y]
            })
            .collect::<Vec<_>>()
    };

    let mut items = Vec::with_capacity(cmds.len());
    let (mut ox, mut oy) = (0.0, 0.0);
    for cmd in cmds {
        let item = match cmd {
            DrawCmd::Line {
                x1,
                y1,
                x2,
                y2,
                w,
                color,
            } => {
                let (x1, y1) = point(ox + x1, oy + y1);
                let (x2, y2) = point(ox + x2, oy + y2);
                NativeCmd::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    w: *w,
                    color: color.clone(),
                }
            }
            DrawCmd::Glyph {
                x,
                y,
                c,
                s,
                a,
                color,
            } => {
                let Some(ch) = char::from_u32(*c) else {
                    continue;
                };
                let v = ch.to_string();
                let (x, y) = text_origin(face.as_ref(), font, ox + x, oy + y, &v, *s, a);
                let (x, y) = point(x, y);
                NativeCmd::Glyphs {
                    x,
                    y,
                    v,
                    s: *s,
                    color: color.clone(),
                }
            }
            DrawCmd::MusicText {
                x,
                y,
                v,
                s,
                a,
                color,
            } => {
                let (x, y) = text_origin(face.as_ref(), font, ox + x, oy + y, v, *s, a);
                let (x, y) = point(x, y);
                NativeCmd::Glyphs {
                    x,
                    y,
                    v: v.clone(),
                    s: *s,
                    color: color.clone(),
                }
            }
            DrawCmd::Text {
                x,
                y,
                v,
                s,
                w,
                i,
                a,
                color,
            } => {
                let (x, y) = point(ox + x, oy + y);
                NativeCmd::Text {
                    x,
                    y,
                    v: v.clone(),
                    s: *s,
                    w: w.clone(),
                    i: *i,
                    a: a.clone(),
                    color: color.clone(),
                }
            }
            DrawCmd::Polygon { pts, color } => NativeCmd::Polygon {
                pts: points(pts, ox, oy),
                color: color.clone(),
            },
            DrawCmd::BezierFill { pts, color } => {
                if pts.len() < 12 {
                    continue;
                }
                NativeCmd::BezierFill {
                    pts: points(&pts[..12], ox, oy),
                    color: color.clone(),
                }
            }
            DrawCmd::Circle { x, y, r, color } => {
                let (x, y) = point(ox + x, oy + y);
                NativeCmd::Circle {
                    x,
                    y,
                    r: *r,
                    color: color.clone(),
                }
            }
            DrawCmd::MoveOrigin { dx, dy } => {
                ox += dx;
                oy += dy;
                continue;
            }
            DrawCmd::FlushContent => continue,
        };
        items.push(item);
    }
    items
}

/// The y-up baseline origin of music text `v` anchored at (x,y) by `anchor`.
fn text_origin(
    face: Option<&Face<'_>>,
    font: glyph::FontId,
    x: f64,
    y: f64,
    v: &str,
    size_mm: f64,
    anchor: &str,
) -> (f64, f64) {
    if let Some(origin) = face.and_then(|face| face_text_origin(face, x, y, v, size_mm, anchor)) {
        return origin;
    }
    let mut chars = v.chars();
    let bbox = match (chars.next(), chars.next()) {
        (Some(ch), None) => smufl_bbox_for_codepoint(ch as u32, font),
        _ => music_text_bbox(v, font),
    };
    match bbox {
        Some(bbox) => anchored_text_origin(x, y, bbox, size_mm, anchor),
        None => (x, y),
    }
}

/// The origin that puts `v`'s ink box, laid out by advance from pen 0,
/// where `anchor_sw` anchors it — what the SVG form does with outlines.
fn face_text_origin(
    face: &Face<'_>,
    x: f64,
    y: f64,
    v: &str,
    size_mm: f64,
    anchor: &str,
) -> Option<(f64, f64)> {
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut pen = 0.0_f64;
    for ch in v.chars() {
        let gid = glyph_id(face, ch)?;
        let bbox = face.glyph_bounding_box(gid)?;
        min_x = min_x.min(pen + bbox.x_min as f64);
        min_y = min_y.min(bbox.y_min as f64);
        max_x = max_x.max(pen + bbox.x_max as f64);
        max_y = max_y.max(bbox.y_max as f64);
        pen += face.glyph_hor_advance(gid).unwrap_or(0) as f64;
    }
    if !min_x.is_finite() {
        return None;
    }
    let scale = size_mm / face.units_per_em() as f64;
    let (sw_x, sw_y) = anchor_sw(
        x,
        y,
        (max_x - min_x) * scale,
        (max_y - min_y) * scale,
        anchor,
    );
    Some((sw_x - min_x * scale, sw_y - min_y * scale))
}

#[cfg(test)]
mod tests {
    use crate::types::{NativeCmd, OutputMode, ScoreInput, ScoreOutput, SystemOutput};
    use ttf_parser::Face;

    /// A two-staff score with beams, a chord, a rest, accidentals, a tie, a
    /// fingering and a per-note colour: one of each item kind it can draw.
    fn input(output: Option<&str>) -> ScoreInput {
        let mut json = serde_json::json!({
            "staves": [
                {"clef": "treble", "music": "c'8n[1]( d'8 e'8 f#'8) color{#ff0000:g'4} <c' e' g'>4 | a'2~v[mf] a'4* r4 | b'1"},
                {"clef": "bass", "music": "c2 g,2 | f,4 e,8 d,8 c,2 | g,,1"}
            ],
            "key": "G",
            "title": "Native",
            "tempo": "Allegro",
            "time": "4/4",
            "staff_group": "grand",
            "staff_size_mm": 1.75,
            "width_mm": 120.0,
            "staff_spacing_mm": 8.0,
            "system_spacing_mm": 12.0,
            "measure_numbers": "none",
            "music_font": "Leland"
        });
        if let Some(mode) = output {
            json["output"] = mode.into();
        }
        serde_json::from_value(json).expect("score input")
    }

    fn render(output: Option<&str>) -> ScoreOutput {
        crate::process_score(&input(output))
    }

    fn pair(svg: &ScoreOutput, native: &ScoreOutput) -> Vec<(SystemOutput, SystemOutput)> {
        assert_eq!(svg.systems.len(), native.systems.len());
        svg.systems
            .iter()
            .cloned()
            .zip(native.systems.iter().cloned())
            .collect()
    }

    fn count(items: &[NativeCmd], kind: fn(&NativeCmd) -> bool) -> usize {
        items.iter().filter(|item| kind(item)).count()
    }

    /// Every `translate(x y)` of the SVG's glyph `<use>`s, in document order.
    fn use_origins(svg: &str) -> Vec<(f64, f64)> {
        svg.split("<use ")
            .skip(1)
            .map(|tail| {
                let at = tail.find("translate(").expect("translate") + "translate(".len();
                let mut nums = tail[at..].split([' ', ')']);
                let x = nums.next().unwrap().parse().unwrap();
                let y = nums.next().unwrap().parse().unwrap();
                (x, y)
            })
            .collect()
    }

    #[test]
    fn svg_stays_the_default_output() {
        assert_eq!(input(None).output, OutputMode::Svg);
        for system in render(None).systems {
            assert!(system.svg.starts_with("<svg"));
            assert!(system.items.is_empty());
            assert_eq!(system.box_width, None);
        }
    }

    #[test]
    fn native_output_carries_items_instead_of_svg() {
        for system in render(Some("native")).systems {
            assert!(system.svg.is_empty());
            assert!(!system.items.is_empty());
        }
    }

    #[test]
    fn a_native_system_takes_the_svg_box() {
        for (svg, native) in pair(&render(None), &render(Some("native"))) {
            let head = &svg.svg[..svg.svg.find('>').unwrap()];
            let w = format!("width=\"{:.3}mm\"", native.box_width.unwrap());
            let h = format!("height=\"{:.3}mm\"", native.box_height.unwrap());
            assert!(head.contains(&w), "{head} lacks {w}");
            assert!(head.contains(&h), "{head} lacks {h}");
        }
    }

    #[test]
    fn native_items_match_the_svg_shapes_one_for_one() {
        for (svg, native) in pair(&render(None), &render(Some("native"))) {
            let items = &native.items;
            let svg = &svg.svg;
            assert_eq!(
                svg.matches("<line ").count(),
                count(items, |i| matches!(i, NativeCmd::Line { .. }))
            );
            assert_eq!(
                svg.matches("<circle ").count(),
                count(items, |i| matches!(i, NativeCmd::Circle { .. }))
            );
            assert_eq!(
                svg.matches("<path d=").count(),
                count(items, |i| matches!(
                    i,
                    NativeCmd::Polygon { .. } | NativeCmd::BezierFill { .. }
                ))
            );
            let glyph_chars: usize = items
                .iter()
                .map(|i| match i {
                    NativeCmd::Glyphs { v, .. } => v.chars().count(),
                    _ => 0,
                })
                .sum();
            assert_eq!(
                svg.matches("<text ").count(),
                count(items, |i| matches!(i, NativeCmd::Text { .. }))
            );
            assert!(glyph_chars > 0);
            assert_eq!(svg.matches("<use ").count(), glyph_chars);
        }
    }

    /// The SVG places each glyph's ink box by its bottom-left corner; a glyph
    /// drawn as text at the native baseline origin must put that corner on
    /// the same point.
    #[test]
    fn native_glyph_origins_put_the_ink_where_the_svg_does() {
        let face = Face::parse(crate::renderer::LELAND_FONT, 0).unwrap();
        for (svg, native) in pair(&render(None), &render(Some("native"))) {
            let mut expected = Vec::new();
            for item in &native.items {
                let NativeCmd::Glyphs { x, y, v, s, .. } = item else {
                    continue;
                };
                let scale = s / face.units_per_em() as f64;
                let mut pen = 0.0;
                for ch in v.chars() {
                    let gid = face.glyph_index(ch).unwrap();
                    let bbox = face.glyph_bounding_box(gid).unwrap();
                    expected.push((
                        x + (pen + bbox.x_min as f64) * scale,
                        y - bbox.y_min as f64 * scale,
                    ));
                    pen += face.glyph_hor_advance(gid).unwrap() as f64;
                }
            }
            let actual = use_origins(&svg.svg);
            assert_eq!(actual.len(), expected.len());
            for ((ax, ay), (ex, ey)) in actual.iter().zip(&expected) {
                assert!((ax - ex).abs() < 0.002, "x {ax} vs {ex}");
                assert!((ay - ey).abs() < 0.002, "y {ay} vs {ey}");
            }
        }
    }

    #[test]
    fn native_text_keeps_its_anchor() {
        let system = &render(Some("native")).systems[0];
        let title = system.items.iter().find_map(|i| match i {
            NativeCmd::Text { v, a, .. } if v == "Native" => Some(a.clone()),
            _ => None,
        });
        assert!(title.is_some_and(|a| !a.is_empty()));
    }

    /// Without a width a system takes its natural width, and its box must still
    /// hold every staff line and barline it draws.
    #[test]
    fn a_natural_width_system_holds_its_lines() {
        for staves in [1, 2] {
            let mut score = input(Some("native"));
            score.staves.truncate(staves);
            score.width_mm = None;
            for system in crate::process_score(&score).systems {
                let width = system.box_width.unwrap();
                for item in &system.items {
                    if let NativeCmd::Line { x1, x2, .. } = item {
                        assert!(
                            x1.max(*x2) <= width,
                            "{staves} staves: line to {} past {width}",
                            x1.max(*x2)
                        );
                    }
                }
            }
        }
    }

    /// A chord opening the music stacks its accidentals in lanes to its left; the music
    /// start must leave them room clear of the clef, on one staff or a braced pair.
    #[test]
    fn a_leading_chord_s_accidentals_clear_the_clef() {
        for staves in [1, 2] {
            let mut score = input(Some("native"));
            score.staves.truncate(staves);
            score.staves[0].music = "<c' c#' d#' f#' g#' a#'>1".into();
            score.key = "C".into();
            score.title = None;
            let system = &crate::process_score(&score).systems[0];
            let codepoint = |v: &str| v.chars().next().map_or(0, u32::from);
            let clef_right = system
                .items
                .iter()
                .filter_map(|i| match i {
                    NativeCmd::Glyphs { x, v, s, .. } if codepoint(v) == 0xE050 => {
                        Some(x + s / 4.0 * 2.6)
                    }
                    _ => None,
                })
                .fold(f64::NEG_INFINITY, f64::max);
            let first_accidental = system
                .items
                .iter()
                .filter_map(|i| match i {
                    NativeCmd::Glyphs { x, v, .. } if (0xE260..=0xE26F).contains(&codepoint(v)) => {
                        Some(*x)
                    }
                    _ => None,
                })
                .fold(f64::INFINITY, f64::min);
            assert!(
                first_accidental > clef_right,
                "{staves} staves: an accidental at {first_accidental} overprints the clef ending at {clef_right}"
            );
        }
    }

    /// A rest sits inside its staff, so a lower staff of rests leaves the gap at the
    /// configured spacing rather than padding it as if the rest reached above the staff.
    #[test]
    fn a_resting_lower_staff_keeps_the_configured_gap() {
        let mut score = input(Some("native"));
        score.title = None;
        score.time = None;
        score.staves[0].music = "g1 |".into();
        score.staves[1].music = "r1 |".into();
        score.staff_spacing_mm = 2.0;
        let system = &crate::process_score(&score).systems[0];
        let mut rules: Vec<f64> = system
            .items
            .iter()
            .filter_map(|i| match i {
                NativeCmd::Line { x1, x2, y1, y2, .. } if y1 == y2 && x2 - x1 > 20.0 => Some(*y1),
                _ => None,
            })
            .collect();
        rules.sort_by(f64::total_cmp);
        rules.dedup();
        let gap = rules[5] - rules[4];
        assert!(
            (gap - score.staff_spacing_mm).abs() < 1e-6,
            "the staves sit {gap}mm apart, configured {}mm",
            score.staff_spacing_mm
        );
    }

    #[test]
    fn native_colours_ride_on_their_items() {
        let system = &render(Some("native")).systems[0];
        let red = system
            .items
            .iter()
            .filter(|i| matches!(i, NativeCmd::Glyphs { color: Some(c), .. } if c == "#ff0000"))
            .count();
        assert!(red > 0);
    }
}
