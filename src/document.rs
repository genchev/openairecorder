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
