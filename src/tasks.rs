use crate::app::GlobalAction;
use chrono::NaiveDate;
use std::cmp::Ordering;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    DueSoonest,
    DueLatest,
    AddedNewest,
    AddedOldest,
}
impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Self::DueSoonest => "Due date · soonest",
            Self::DueLatest => "Due date · latest",
            Self::AddedNewest => "Added date · newest",
            Self::AddedOldest => "Added date · oldest",
        }
    }
}
pub fn parse_due(text: &str) -> Option<NaiveDate> {
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    (date.format("%Y-%m-%d").to_string() == text).then_some(date)
}
pub fn due(g: &GlobalAction) -> Option<NaiveDate> {
    g.action.due.as_deref().and_then(parse_due)
}
pub fn days_until(g: &GlobalAction, today: NaiveDate) -> Option<i64> {
    due(g).map(|date| date.signed_duration_since(today).num_days())
}
pub fn priority(g: &GlobalAction, today: NaiveDate) -> u8 {
    if g.action.done {
        return 5;
    }
    match days_until(g, today) {
        Some(days) if days < 0 => 0,
        Some(0) => 1,
        Some(1) => 2,
        _ if g.pinned => 3,
        _ => 4,
    }
}
pub fn deadline_label(g: &GlobalAction, today: NaiveDate) -> Option<String> {
    let date = g.action.due.as_ref()?;
    let label = if g.action.done {
        "Due".into()
    } else {
        match days_until(g, today) {
            Some(days) if days < 0 => format!(
                "Overdue · {} day{}",
                -days,
                if days == -1 { "" } else { "s" }
            ),
            Some(0) => "Due today".into(),
            Some(1) => "Due tomorrow".into(),
            Some(_) => "Due".into(),
            None => "Suggested deadline · set a date for reminders".into(),
        }
    };
    Some(format!("{label} · {date}"))
}
fn optional_cmp<T: Ord>(a: Option<T>, b: Option<T>, reverse: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            if reverse {
                b.cmp(&a)
            } else {
                a.cmp(&b)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}
pub fn sort(actions: &mut [GlobalAction], today: NaiveDate, order: Sort) {
    actions.sort_by(|a, b| {
        priority(a, today)
            .cmp(&priority(b, today))
            .then_with(|| match order {
                Sort::DueSoonest | Sort::DueLatest => {
                    optional_cmp(due(a), due(b), order == Sort::DueLatest)
                }
                Sort::AddedNewest | Sort::AddedOldest => optional_cmp(
                    a.added_at
                        .as_deref()
                        .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok()),
                    b.added_at
                        .as_deref()
                        .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok()),
                    order == Sort::AddedNewest,
                ),
            })
            .then_with(|| {
                a.action
                    .title
                    .to_lowercase()
                    .cmp(&b.action.title.to_lowercase())
            })
            .then(a.id.cmp(&b.id))
    });
}
pub fn matches(g: &GlobalAction, query: &str) -> bool {
    format!(
        "{} {} {} {}",
        g.action.title,
        g.action.owner.as_deref().unwrap_or(""),
        g.action.due.as_deref().unwrap_or(""),
        g.source_title
    )
    .to_lowercase()
    .contains(&query.trim().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Action;
    use uuid::Uuid;
    fn task(
        title: &str,
        date: Option<&str>,
        pinned: bool,
        done: bool,
        added: Option<&str>,
    ) -> GlobalAction {
        GlobalAction {
            id: Uuid::new_v4(),
            source_session: Uuid::nil(),
            source_action: Uuid::nil(),
            source_title: "Планиране".into(),
            action: Action {
                id: Uuid::new_v4(),
                title: title.into(),
                owner: Some("Мария".into()),
                due: date.map(String::from),
                done,
            },
            added_at: added.map(String::from),
            pinned,
        }
    }
    #[test]
    fn deadline_boundaries_follow_calendar_days_and_completed_tasks_are_not_urgent() {
        let today = parse_due("2026-12-31").unwrap();
        for (date, rank, label) in [
            ("2026-12-30", 0, "Overdue"),
            ("2026-12-31", 1, "Due today"),
            ("2027-01-01", 2, "Due tomorrow"),
            ("2027-01-02", 4, "Due"),
        ] {
            let g = task("Task", Some(date), false, false, None);
            assert_eq!(priority(&g, today), rank);
            assert!(deadline_label(&g, today).unwrap().starts_with(label));
        }
        assert_eq!(
            priority(&task("Done", Some("2026-12-01"), true, true, None), today),
            5
        );
        assert_eq!(priority(&task("Pinned", None, true, false, None), today), 3);
        assert_eq!(
            priority(&task("Unknown", Some("Friday"), false, false, None), today),
            4
        );
        assert!(parse_due("2026-02-29").is_none());
        assert!(parse_due("2026-2-01").is_none());
        assert!(parse_due("2028-02-29").is_some());
        // Tomorrow becomes due today, then overdue, without mutating persisted tasks.
        let g = task("Rollover", Some("2027-01-01"), false, false, None);
        assert_eq!(priority(&g, today), 2);
        assert_eq!(priority(&g, parse_due("2027-01-01").unwrap()), 1);
        assert_eq!(priority(&g, parse_due("2027-01-02").unwrap()), 0);
    }
    #[test]
    fn sorting_prioritizes_urgency_pins_and_open_state_in_every_sort_mode() {
        let today = parse_due("2026-10-15").unwrap();
        let original = vec![
            task("Done", Some("2026-10-01"), true, true, None),
            task("Ordinary", Some("2026-11-01"), false, false, None),
            task("Pinned", None, true, false, None),
            task("Tomorrow", Some("2026-10-16"), false, false, None),
            task("Today", Some("2026-10-15"), false, false, None),
            task("Late", Some("2026-10-14"), false, false, None),
        ];
        for order in [
            Sort::DueSoonest,
            Sort::DueLatest,
            Sort::AddedNewest,
            Sort::AddedOldest,
        ] {
            let mut list = original.clone();
            sort(&mut list, today, order);
            assert_eq!(
                list.iter()
                    .map(|g| g.action.title.as_str())
                    .collect::<Vec<_>>(),
                vec!["Late", "Today", "Tomorrow", "Pinned", "Ordinary", "Done"]
            );
        }
        let mut list = vec![
            task(
                "Early",
                Some("2026-11-01"),
                false,
                false,
                Some("2026-10-01T12:00:00Z"),
            ),
            task(
                "Later",
                Some("2026-12-01"),
                false,
                false,
                Some("2026-10-02T12:00:00Z"),
            ),
            task("Unknown", None, false, false, None),
        ];
        for (order, first) in [
            (Sort::DueSoonest, "Early"),
            (Sort::DueLatest, "Later"),
            (Sort::AddedNewest, "Later"),
            (Sort::AddedOldest, "Early"),
        ] {
            sort(&mut list, today, order);
            assert_eq!(list[0].action.title, first);
            assert_eq!(list[2].action.title, "Unknown");
        }
    }
    #[test]
    fn task_search_covers_unicode_titles_owners_source_and_dates() {
        let g = task("Изпрати офертата", Some("2026-10-20"), false, false, None);
        for query in ["ОФЕРТАТА", "мария", "ПЛАНИРАНЕ", "2026-10", "   "] {
            assert!(matches(&g, query));
        }
        assert!(!matches(&g, "nonexistent"));
    }
}
