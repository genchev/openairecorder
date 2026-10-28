//! Read-only, Unicode-aware search of the text rendered from Markdown.
use pulldown_cmark::{Event, Options, Parser, TagEnd};
use std::{
    hash::{Hash, Hasher},
    ops::Range,
    sync::Arc,
};

pub struct Source {
    pub label: &'static str,
    pub markdown: String,
    pub is_markdown: bool,
}
#[derive(Clone)]
pub struct Hit {
    pub source_index: usize,
    pub occurrence: usize,
    pub source: &'static str,
    pub text: Arc<str>,
    pub range: Range<usize>,
}
#[derive(Default)]
pub struct Search {
    pub query: String,
    pub index: usize,
    pub hits: Vec<Hit>,
    pub jump: Option<crate::search_jump::Jump>,
    previous_query: String,
    fingerprint: Option<u64>,
}
fn plain(markdown: &str) -> String {
    let mut text = String::new();
    for event in Parser::new_ext(markdown, Options::ENABLE_TASKLISTS) {
        match event {
            Event::Text(s) | Event::Code(s) => text.push_str(&s),
            Event::SoftBreak
            | Event::HardBreak
            | Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::TableCell,
            ) => text.push(' '),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub(crate) fn ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if query.is_empty() {
        return vec![];
    }
    let mut lower = String::new();
    let mut offsets = Vec::new();
    for (start, c) in text.char_indices() {
        for folded in c.to_lowercase() {
            lower.push(folded);
            offsets.extend(std::iter::repeat_n(
                start..start + c.len_utf8(),
                folded.len_utf8(),
            ));
        }
    }
    lower
        .match_indices(&query)
        .map(|(start, _)| offsets[start].start..offsets[start + query.len() - 1].end)
        .collect()
}
impl Search {
    pub fn refresh(&mut self, sources: &[Source]) {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.query.hash(&mut hash);
        for source in sources {
            source.label.hash(&mut hash);
            source.markdown.hash(&mut hash);
            source.is_markdown.hash(&mut hash);
        }
        let fingerprint = hash.finish();
        if self.fingerprint == Some(fingerprint) {
            return;
        }
        if self.previous_query != self.query {
            self.index = 0;
        }
        self.jump = None;
        self.hits.clear();
        for (source_index, source) in sources.iter().enumerate() {
            let text: Arc<str> = if source.is_markdown {
                plain(&source.markdown).into()
            } else {
                source.markdown.clone().into()
            };
            for (occurrence, range) in ranges(&text, &self.query).into_iter().enumerate() {
                self.hits.push(Hit {
                    source_index,
                    occurrence,
                    source: source.label,
                    text: text.clone(),
                    range,
                });
            }
        }
        self.index = self.index.min(self.hits.len().saturating_sub(1));
        self.previous_query = self.query.clone();
        self.fingerprint = Some(fingerprint);
    }
    pub fn request_jump(&mut self, now: f64) {
        self.jump = self
            .hits
            .get(self.index)
            .cloned()
            .map(|hit| crate::search_jump::Jump::new(hit, now));
    }
    pub fn step(&mut self, forward: bool) {
        self.jump = None;
        if !self.hits.is_empty() {
            self.index = if forward {
                (self.index + 1) % self.hits.len()
            } else {
                (self.index + self.hits.len() - 1) % self.hits.len()
            };
        }
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}
impl Hit {
    pub fn excerpt(&self) -> (&str, &str, &str, bool, bool) {
        let start = self.text[..self.range.start]
            .char_indices()
            .rev()
            .nth(90)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let end = self.text[self.range.end..]
            .char_indices()
            .nth(90)
            .map(|(i, _)| self.range.end + i)
            .unwrap_or(self.text.len());
        (
            &self.text[start..self.range.start],
            &self.text[self.range.clone()],
            &self.text[self.range.end..end],
            start > 0,
            end < self.text.len(),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_matching_maps_original_bytes_even_when_case_conversion_expands() {
        let text = "🎤 България İSTANBUL — БЪЛГАРИЯ";
        let found = ranges(text, "българия");
        assert_eq!(found.len(), 2);
        assert_eq!(&text[found[0].clone()], "България");
        assert_eq!(&text[found[1].clone()], "БЪЛГАРИЯ");
        assert_eq!(&text[ranges(text, "i")[0].clone()], "İ");
        assert!(ranges(text, "  ").is_empty());
        assert_eq!(plain("об**съж**дане на `срещата`"), "обсъждане на срещата");
    }
    #[test]
    fn rendered_markdown_search_navigation_and_live_refresh_preserve_the_source() {
        let original = "## Key points\n- **Мария** изпраща офертата.\n- Мария проверява `теста`.\n";
        let mut search = Search {
            query: "мария".into(),
            ..Default::default()
        };
        let mut sources = vec![
            Source {
                label: "Notes",
                markdown: original.into(),
                is_markdown: true,
            },
            Source {
                label: "Action item",
                markdown: "Call Мария".into(),
                is_markdown: false,
            },
        ];
        search.refresh(&sources);
        assert_eq!(search.hits.len(), 3);
        assert_eq!(search.hits[2].source, "Action item");
        assert_eq!(search.hits[0].excerpt().1, "Мария");
        search.step(false);
        assert_eq!(search.index, 2);
        search.step(true);
        assert_eq!(search.index, 0);
        search.step(true);
        sources[0].markdown.push_str("\nМария adds another point.");
        search.refresh(&sources);
        assert_eq!(search.hits.len(), 4);
        assert_eq!(search.index, 1);
        search.query = "изпраща офертата".into();
        search.refresh(&sources);
        assert_eq!(search.hits.len(), 1);
        assert_eq!(search.index, 0);
        assert!(sources[0].markdown.starts_with(original));
        search.query = "absent".into();
        search.refresh(&sources);
        assert!(search.hits.is_empty());
        search.step(true);
        search.clear();
        assert!(search.query.is_empty());
    }
}
