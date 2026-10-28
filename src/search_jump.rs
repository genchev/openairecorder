//! Locate a selected search hit in rendered glyph geometry without changing Markdown.
use crate::session_search::Hit;
use eframe::egui::{self, Pos2, Rect, Shape};

#[derive(Clone)]
pub struct Glyph {
    pub chr: char,
    pub rect: Rect,
}
pub struct Jump {
    pub target: Hit,
    until: f64,
    pending: bool,
    width: f32,
    rectangles: Vec<Rect>,
}
impl Jump {
    pub fn new(target: Hit, now: f64) -> Self {
        Self {
            target,
            until: now + 3.0,
            pending: true,
            width: 0.0,
            rectangles: vec![],
        }
    }
    pub fn active(&self, ui: &egui::Ui) -> bool {
        self.until > ui.input(|i| i.time)
    }
    pub fn needs_geometry(&self, ui: &egui::Ui, width: f32) -> bool {
        self.active(ui) && (self.pending || (self.width - width).abs() > 1.0)
    }
    pub fn finish(
        &mut self,
        ui: &mut egui::Ui,
        origin: Pos2,
        width: f32,
        slot: egui::layers::ShapeIdx,
        glyphs: &[Glyph],
    ) {
        if !self.active(ui) {
            return;
        }
        if self.needs_geometry(ui, width) {
            self.rectangles = locate(&self.target, glyphs)
                .into_iter()
                .map(|r| r.translate(-origin.to_vec2()))
                .collect();
            self.width = width;
        }
        let rects: Vec<_> = self
            .rectangles
            .iter()
            .map(|r| r.translate(origin.to_vec2()))
            .collect();
        if self.pending
            && let Some(first) = rects.first()
        {
            ui.scroll_to_rect(*first, Some(egui::Align::Center));
        }
        self.pending = false;
        ui.painter().set(
            slot,
            Shape::Vec(
                rects
                    .into_iter()
                    .map(|r| {
                        Shape::rect_filled(
                            r.expand(1.0),
                            2,
                            egui::Color32::from_rgba_unmultiplied(237, 198, 110, 100),
                        )
                    })
                    .collect(),
            ),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    pub fn row(&mut self, ui: &mut egui::Ui, rect: Rect) {
        if !self.active(ui) {
            return;
        }
        if self.pending {
            ui.scroll_to_rect(rect, Some(egui::Align::Center));
            self.pending = false;
        }
        ui.painter().rect_stroke(
            rect.expand(3.0),
            5,
            egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(237, 198, 110)),
            egui::StrokeKind::Inside,
        );
    }
}
/// A disabled measurement pass renders even clipped text, then discards all its
/// paint. The visible viewer and its controls render normally afterwards.
pub fn capture(
    ui: &mut egui::Ui,
    cache: &mut egui_commonmark::CommonMarkCache,
    markdown: &str,
    output: &mut Vec<Glyph>,
) {
    let layer = ui.layer_id();
    let start = ui.ctx().graphics_mut(|g| g.entry(layer).next_idx().0);
    let mut measure = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("search-measure", start))
            .max_rect(ui.available_rect_before_wrap())
            .layout(*ui.layout())
            .disabled(),
    );
    measure.set_clip_rect(Rect::EVERYTHING);
    egui_commonmark::CommonMarkViewer::new().show(&mut measure, cache, markdown);
    ui.ctx().graphics_mut(|graphics| {
        let list = graphics.entry(layer);
        let end = list.next_idx().0;
        for entry in list.all_entries().skip(start) {
            collect(&entry.shape, output);
        }
        for i in start..end {
            list.reset_shape(egui::layers::ShapeIdx(i));
        }
    });
}
fn collect(shape: &Shape, output: &mut Vec<Glyph>) {
    match shape {
        Shape::Text(text) => {
            // Ordered-list markers are decoration, not searchable source text.
            let job = text.galley.text();
            if job.ends_with('.') && job[..job.len() - 1].chars().all(|c| c.is_ascii_digit()) {
                return;
            }
            let first = text
                .galley
                .rows
                .iter()
                .flat_map(|r| r.glyphs.iter().map(move |g| (r, g)))
                .next();
            if let (Some(last), Some((row, g))) = (output.last(), first) {
                let pos = text.pos + row.pos.to_vec2() + g.pos.to_vec2();
                if !last.chr.is_whitespace()
                    && (pos.x - last.rect.right() > 1.5
                        || (pos.y - last.rect.bottom()).abs() > g.font_height * 0.6)
                {
                    output.push(Glyph {
                        chr: ' ',
                        rect: Rect::NOTHING,
                    });
                }
            }
            for row in &text.galley.rows {
                for g in &row.glyphs {
                    let base = text.pos + row.pos.to_vec2() + g.pos.to_vec2();
                    output.push(Glyph {
                        chr: g.chr,
                        rect: Rect::from_min_size(
                            egui::pos2(base.x, base.y - g.font_ascent),
                            egui::vec2(g.advance_width, g.font_height),
                        ),
                    });
                }
            }
        }
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, output);
            }
        }
        _ => {}
    }
}
fn locate(target: &Hit, glyphs: &[Glyph]) -> Vec<Rect> {
    let mut text = String::new();
    let mut positions = Vec::new();
    for glyph in glyphs {
        if glyph.chr.is_whitespace() {
            if text.is_empty() || text.ends_with(' ') {
                continue;
            }
            text.push(' ');
            positions.push(glyph.rect);
        } else {
            text.push(glyph.chr);
            positions.extend(std::iter::repeat_n(glyph.rect, glyph.chr.len_utf8()));
        }
    }
    let candidates = crate::session_search::ranges(&text, &target.text[target.range.clone()]);
    let before: Vec<_> = target.text[..target.range.start]
        .chars()
        .rev()
        .take(100)
        .collect();
    let after: Vec<_> = target.text[target.range.end..].chars().take(100).collect();
    let direct = text.trim() == target.text.as_ref();
    let range = if direct {
        let offset = text.len() - text.trim_start().len();
        target.range.start + offset..target.range.end + offset
    } else {
        let best = candidates.iter().enumerate().max_by_key(|(i, r)| {
            let prefix = text[..r.start]
                .chars()
                .rev()
                .zip(before.iter())
                .take_while(|(a, b)| a == *b)
                .count();
            let suffix = text[r.end..]
                .chars()
                .zip(after.iter())
                .take_while(|(a, b)| a == *b)
                .count();
            (
                prefix + suffix,
                std::cmp::Reverse(i.abs_diff(target.occurrence)),
            )
        });
        let Some((_, range)) = best else {
            return vec![];
        };
        range.clone()
    };
    let mut rects: Vec<Rect> = Vec::new();
    for rect in &positions[range.clone()] {
        if !rect.is_finite() || !rect.is_positive() {
            continue;
        }
        if let Some(last) = rects.last_mut()
            && (last.center().y - rect.center().y).abs() < 2.0
            && rect.left() - last.right() < 3.0
        {
            *last = last.union(*rect);
        } else {
            rects.push(*rect);
        }
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_search::{Search, Source};
    fn glyphs(text: &str) -> Vec<Glyph> {
        text.chars()
            .enumerate()
            .map(|(i, chr)| Glyph {
                chr,
                rect: Rect::from_min_size(egui::pos2(i as f32, 0.0), egui::vec2(1.0, 12.0)),
            })
            .collect()
    }
    #[test]
    fn selected_occurrence_and_unicode_highlight_use_the_correct_glyphs() {
        for text in [
            "release one release two release three",
            "Мария първо Мария после Мария накрая",
        ] {
            let query = if text.starts_with("release") {
                "release"
            } else {
                "мария"
            };
            let mut search = Search::default();
            search.query = query.into();
            search.refresh(&[Source {
                label: "Notes",
                markdown: text.into(),
                is_markdown: false,
            }]);
            for hit in &search.hits {
                let rects = locate(hit, &glyphs(text));
                assert_eq!(
                    rects[0].left(),
                    text[..hit.range.start].chars().count() as f32
                );
                assert_eq!(
                    rects.last().unwrap().right(),
                    text[..hit.range.end].chars().count() as f32
                );
            }
        }
    }
    #[test]
    fn offscreen_wrapped_markdown_can_be_located_without_painting_or_editing_it() {
        let markdown = format!(
            "## Notes\nNeedle at the start.\n\n{}\n\n**Needle** at the end.",
            "Ordinary wrapped text. ".repeat(120)
        );
        let mut search = Search::default();
        search.query = "needle".into();
        search.refresh(&[Source {
            label: "Transcript",
            markdown: markdown.clone(),
            is_markdown: true,
        }]);
        let ctx = egui::Context::default();
        let mut captured = vec![];
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 200.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut offscreen = ui.new_child(egui::UiBuilder::new().max_rect(
                        Rect::from_min_size(egui::pos2(0.0, 2000.0), egui::vec2(280.0, 10000.0)),
                    ));
                    let mut cache = egui_commonmark::CommonMarkCache::default();
                    capture(&mut offscreen, &mut cache, &markdown, &mut captured);
                    assert!(
                        ctx.graphics(|g| g
                            .get(ui.layer_id())
                            .unwrap()
                            .all_entries()
                            .all(|e| !matches!(e.shape, Shape::Text(_)))),
                        "measurement text must not remain in visible paint"
                    );
                });
            },
        );
        let first = locate(&search.hits[0], &captured);
        let last = locate(&search.hits[1], &captured);
        assert!(!first.is_empty() && !last.is_empty());
        assert!(first[0].top() >= 2000.0);
        assert!(last[0].top() > first[0].top() + 500.0);
        assert!(markdown.contains("**Needle**"));
    }
}
