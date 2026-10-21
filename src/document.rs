//! Read returned Markdown without interpreting speech or inventing actions.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Debug, Clone)]
pub struct Task {
    pub title: String,
    pub done: bool,
    pub marker: Range<usize>,
}
pub fn tasks(markdown: &str) -> Vec<Task> {
    let mut items: Vec<Option<Task>> = Vec::new();
    let mut result = Vec::new();
    for (event, range) in Parser::new_ext(markdown, Options::ENABLE_TASKLISTS).into_offset_iter() {
        match event {
            Event::Start(Tag::Item) => items.push(None),
            Event::TaskListMarker(done) => {
                if let Some(item) = items.last_mut() {
                    *item = Some(Task {
                        title: String::new(),
                        done,
                        marker: range,
                    });
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(Some(task)) = items.last_mut() {
                    task.title.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(Some(task)) = items.last_mut() {
                    task.title.push(' ');
                }
            }
            Event::End(TagEnd::Item) => {
                if let Some(Some(mut task)) = items.pop() {
                    task.title = task.title.trim().to_owned();
                    if !task.title.is_empty() {
                        result.push(task);
                    }
                }
            }
            _ => {}
        }
    }
    result.sort_by_key(|task| task.marker.start);
    result
}
pub fn sync_tasks(markdown: &str, checked: impl Fn(&str) -> Option<bool>) -> String {
    let mut result = markdown.to_owned();
    for task in tasks(markdown).into_iter().rev() {
        if let Some(done) = checked(&task.title) {
            result.replace_range(task.marker, if done { "[x]" } else { "[ ]" });
        }
    }
    result
}
pub fn transcript(markdown: &str) -> String {
    let mut heading_start = None;
    let mut heading_text = String::new();
    let mut capture = None;
    for (event, range) in Parser::new_ext(markdown, Options::ENABLE_TASKLISTS).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if let Some((start, depth)) = capture
                    && level <= depth
                {
                    return markdown[start..range.start].trim().to_owned();
                }
                heading_start = Some(level);
                heading_text.clear();
            }
            Event::Text(text) if heading_start.is_some() => heading_text.push_str(&text),
            Event::End(TagEnd::Heading(_)) => {
                if let Some(depth) = heading_start.take() {
                    let name = heading_text.trim().to_lowercase();
                    if [
                        "transcript",
                        "full transcript",
                        "транскрипция",
                        "пълен препис",
                        "препис",
                        "пълна транскрипция",
                    ]
                    .contains(&name.as_str())
                    {
                        capture = Some((range.end, depth));
                    }
                }
            }
            _ => {}
        }
    }
    capture
        .map(|(start, _)| markdown[start..].trim().to_owned())
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bulgarian_tasks_formatting_code_and_completion_are_preserved() {
        let md = "## Действия\n- [ ] **Мария** да изпрати `офертата`\n- [x] Готово\n\n```md\n- [ ] not an action\n```\n\n## Пълен препис\n[00:01] Здравейте. [неясно]\n\n## Decisions\nNone\n";
        let parsed = tasks(md);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].title, "Мария да изпрати офертата");
        assert!(parsed[1].done);
        let synced = sync_tasks(md, |title| (title == parsed[0].title).then_some(true));
        assert!(synced.contains("- [x] **Мария**"));
        assert!(synced.contains("- [ ] not an action"));
        assert_eq!(transcript(md), "[00:01] Здравейте. [неясно]");
    }
    #[test]
    fn ordinary_bullets_do_not_become_invented_tasks() {
        assert!(tasks("## Actions\n- Discussed hiring\n\nNo commitments.").is_empty());
        assert!(transcript("## Summary\nЗдравейте").is_empty());
    }
}

#[derive(Debug)]
pub struct NotePoint {
    pub title: String,
    pub range: Range<usize>,
}
pub fn action_title(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
/// Top-level bullet/numbered items in the three promotable note sections.
/// Markdown offsets preserve formatting, links and nested lists in the viewer.
pub fn note_points(markdown: &str) -> Vec<NotePoint> {
    let mut heading = None;
    let mut heading_text = String::new();
    let mut section = None;
    let mut depth: usize = 0;
    let mut point: Option<NotePoint> = None;
    let mut result = Vec::new();
    for (event, range) in Parser::new_ext(markdown, Options::ENABLE_TASKLISTS).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if section.is_some_and(|old| level <= old) {
                    section = None;
                }
                heading = Some(level);
                heading_text.clear();
            }
            Event::Text(text) | Event::Code(text) if heading.is_some() => {
                heading_text.push_str(&text)
            }
            Event::End(TagEnd::Heading(_)) => {
                let name = heading_text.trim().trim_end_matches(':').to_lowercase();
                if [
                    "key points",
                    "decisions",
                    "open questions",
                    "ключови точки",
                    "основни точки",
                    "ключови моменти",
                    "основни моменти",
                    "ключови изводи",
                    "основни изводи",
                    "взети решения",
                    "решения",
                    "отворени въпроси",
                    "нерешени въпроси",
                ]
                .contains(&name.as_str())
                {
                    section = heading;
                }
                heading = None;
            }
            Event::Start(Tag::List(_)) if point.is_some() => {
                point.as_mut().unwrap().title.push(' ');
            }
            Event::Start(Tag::Item) => {
                if depth == 0 && section.is_some() {
                    point = Some(NotePoint {
                        title: String::new(),
                        range: range.clone(),
                    });
                }
                depth += 1;
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(p) = &mut point {
                    p.title.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak | Event::End(TagEnd::Paragraph) => {
                if let Some(p) = &mut point {
                    p.title.push(' ');
                }
            }
            Event::End(TagEnd::Item) => {
                depth = depth.saturating_sub(1);
                if depth > 0
                    && let Some(p) = &mut point
                {
                    p.title.push(' ');
                }
                if depth == 0
                    && let Some(mut p) = point.take()
                {
                    p.title = p.title.split_whitespace().collect::<Vec<_>>().join(" ");
                    p.range.end = range.end;
                    if !p.title.is_empty() {
                        result.push(p);
                    }
                }
            }
            _ => {}
        }
    }
    result
}
pub fn append_action(markdown: &str, title: &str) -> String {
    // Escape plain text so source punctuation cannot inject headings or checkboxes.
    let escaped: String = title
        .chars()
        .flat_map(|c| {
            if c.is_ascii_punctuation() {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect();
    let mut heading = None;
    let mut text = String::new();
    let mut section = None;
    let mut insertion = markdown.len();
    for (event, range) in Parser::new(markdown).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if section.is_some_and(|depth| level <= depth) {
                    insertion = range.start;
                    break;
                }
                heading = Some(level);
                text.clear();
            }
            Event::Text(t) | Event::Code(t) if heading.is_some() => text.push_str(&t),
            Event::End(TagEnd::Heading(_)) => {
                if ["action items", "actions", "действия", "задачи"]
                    .contains(&text.trim().trim_end_matches(':').to_lowercase().as_str())
                {
                    section = heading;
                }
                heading = None;
            }
            _ => {}
        }
    }
    if section.is_some() {
        format!(
            "{}\n- [ ] {}\n\n{}",
            markdown[..insertion].trim_end(),
            escaped,
            &markdown[insertion..]
        )
    } else {
        format!(
            "{}\n\n## Action items\n- [ ] {}\n",
            markdown.trim_end(),
            escaped
        )
    }
}

#[cfg(test)]
mod point_tests {
    use super::*;
    #[test]
    fn points_use_target_sections_and_preserve_markdown_offsets_and_bulgarian() {
        let md = "## Summary\n- Not eligible\n## **Key Points**\n- **Важна** точка с `код`.\n- Second point\n## Decisions\n1. Ship [the app](https://example.com).\n## Open Questions\n- Who owns this?\n## Other\n- Not eligible either\n```md\n## Decisions\n- Fake point\n```\n";
        let p = note_points(md);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].title, "Важна точка с код.");
        assert_eq!(p[2].title, "Ship the app.");
        for pair in p.windows(2) {
            assert!(pair[0].range.end <= pair[1].range.start);
        }
        assert!(md[p[0].range.clone()].contains("**Важна**"));
        assert_eq!(
            note_points("## Решения\n- Да изпратим офертата\n")[0].title,
            "Да изпратим офертата"
        );
    }
    #[test]
    fn appended_tasks_are_literal_and_reuse_action_section_without_touching_other_sections() {
        let original = "## Key Points\n- Keep this.\n## Action items\n- [x] Existing\n## Open Questions\n- Who?\n";
        let title = "Review [link] and `code` *today*.";
        let result = append_action(original, title);
        assert_eq!(result.matches("## Action items").count(), 1);
        assert!(result.ends_with("## Open Questions\n- Who?\n"));
        let parsed = tasks(&result);
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].done);
        assert_eq!(parsed[1].title, title);
        let again = append_action(&result, "Another");
        assert_eq!(again.matches("## Action items").count(), 1);
        assert_eq!(tasks(&again).len(), 3);
    }
}
