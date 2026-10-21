use crate::overlay::Overlay;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State as Extract},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone)]
pub struct State {
    pub root: PathBuf,
    pub overlay: Overlay,
    pub jobs: Arc<Mutex<()>>,
    pub gemini: Arc<crate::gemini::Service>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Recording {
    pub id: Uuid,
    pub title: String,
    pub created_at: String,
    pub status: String,
    pub transcript: String,
    #[serde(default)]
    pub gemini_processed: u64,
    pub notes: String,
    pub error: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub note_actions: std::collections::HashMap<String, Uuid>,
    #[serde(default = "default_audio_type")]
    pub audio_type: String,
    #[serde(default)]
    pub actions: Vec<Action>,
}
fn apply_document(r: &mut Recording, document: String, preserve_completion: bool) {
    let mut actions = Vec::new();
    for task in crate::document::tasks(&document) {
        if actions.iter().any(|a: &Action| a.title == task.title) {
            continue;
        }
        let old = r.actions.iter().find(|a| a.title == task.title);
        actions.push(Action {
            id: old.map(|a| a.id).unwrap_or_else(Uuid::new_v4),
            title: task.title,
            owner: old.and_then(|a| a.owner.clone()),
            due: old.and_then(|a| a.due.clone()),
            done: task.done || preserve_completion && old.is_some_and(|a| a.done),
        });
    }
    let transcript = crate::document::transcript(&document);
    if !transcript.is_empty() {
        r.transcript = transcript;
    }
    r.actions = actions;
    r.notes = crate::document::sync_tasks(&document, |title| {
        r.actions.iter().find(|a| a.title == title).map(|a| a.done)
    });
    r.status = "ready".into();
    r.error = None;
}
type Error = (StatusCode, String);
#[derive(Serialize, Deserialize)]
struct Settings {
    theme: String,
}
async fn get_settings(Extract(s): Extract<State>) -> Result<Json<Settings>, Error> {
    match tokio::fs::read(s.root.join("settings.json")).await {
        Ok(bytes) => Ok(Json(serde_json::from_slice(&bytes).map_err(internal)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Json(Settings {
            theme: "system".into(),
        })),
        Err(e) => Err(internal(e)),
    }
}
async fn set_settings(
    Extract(s): Extract<State>,
    Json(input): Json<Settings>,
) -> Result<StatusCode, Error> {
    if !["light", "dark", "system"].contains(&input.theme.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Choose Light, Dark, or System".into(),
        ));
    }
    let _lock = s.jobs.lock().await;
    tokio::fs::write(
        s.root.join("settings.json.tmp"),
        serde_json::to_vec(&input).map_err(internal)?,
    )
    .await
    .map_err(internal)?;
    tokio::fs::rename(
        s.root.join("settings.json.tmp"),
        s.root.join("settings.json"),
    )
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Action {
    pub id: Uuid,
    pub title: String,
    pub owner: Option<String>,
    pub due: Option<String>,
    pub done: bool,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct GlobalAction {
    pub id: Uuid,
    pub source_session: Uuid,
    pub source_title: String,
    pub source_action: Uuid,
    pub action: Action,
    #[serde(default)]
    pub added_at: Option<String>,
    #[serde(default)]
    pub pinned: bool,
}

pub(crate) mod desktop_api {
    use super::*;
    #[derive(Default)]
    pub struct Snapshot {
        pub sessions: Vec<Recording>,
        pub actions: Vec<GlobalAction>,
        pub theme: String,
    }
    pub enum Command {
        GeminiSettings {
            settings: crate::gemini::Settings,
            key: String,
        },
        #[cfg_attr(not(windows), allow(dead_code))]
        FinishLive {
            id: Uuid,
            total: u64,
            warning: Option<String>,
        },
        ProcessGemini(Uuid),
        RetryGemini(Uuid),
        Import {
            path: PathBuf,
            title: String,
            owned_capture: bool,
        },
        ImportMarkdown {
            id: Uuid,
            path: PathBuf,
        },
        Notes {
            id: Uuid,
            notes: String,
        },
        Pin {
            id: Uuid,
            pinned: bool,
        },
        Complete {
            id: Uuid,
            action: Uuid,
            done: bool,
        },
        Copy {
            id: Uuid,
            action: Uuid,
        },
        GlobalComplete {
            id: Uuid,
            done: bool,
        },
        ActionFromNote {
            id: Uuid,
            title: String,
        },
        GlobalSave {
            id: Option<Uuid>,
            title: String,
            owner: String,
            due: Option<String>,
        },
        GlobalPin {
            id: Uuid,
            pinned: bool,
        },
        GlobalRemove(Uuid),
        Archive {
            id: Uuid,
            archived: bool,
        },
        Delete(Uuid),
        Theme(String),
        Playback(Uuid),
    }
    pub struct Outcome {
        pub session: Option<Uuid>,
        pub file: Option<PathBuf>,
        pub message: String,
    }
    pub async fn recover(s: &State) -> Result<(usize, usize), String> {
        let mut entries = tokio::fs::read_dir(&s.root)
            .await
            .map_err(|e| e.to_string())?;
        let mut recovered = 0;
        let mut retained = 0;
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            let path = entry.path();
            if !path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".capture-"))
                || path.extension().is_none_or(|e| e != "wav")
            {
                continue;
            }
            if !hound::WavReader::open(&path).is_ok_and(|r| r.duration() > 0) {
                retained += 1;
                continue;
            }
            match execute(
                s,
                Command::Import {
                    path,
                    title: "Recovered recording".into(),
                    owned_capture: true,
                },
            )
            .await
            {
                Ok(_) => recovered += 1,
                Err(_) => retained += 1,
            }
        }
        Ok((recovered, retained))
    }
    pub async fn snapshot(s: &State) -> Result<Snapshot, String> {
        s.prune_archives(chrono::Utc::now())
            .await
            .map_err(|e| e.to_string())?;
        {
            use std::hash::{Hash, Hasher};
            let _lock = s.jobs.lock().await;
            for mut r in s.list().await.map_err(|e| e.to_string())? {
                if r.status == "processing" || r.status == "recording" {
                    continue;
                }
                let dir = s.root.join(r.id.to_string());
                let returned = dir.join("agent-output.md");
                if !tokio::fs::metadata(&returned)
                    .await
                    .is_ok_and(|m| m.len() > 0 && m.len() <= 1_000_000)
                {
                    continue;
                }
                let Ok(document) = tokio::fs::read_to_string(returned).await else {
                    continue;
                };
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                document.hash(&mut hasher);
                let hash = format!("{:016x}", hasher.finish());
                if tokio::fs::read_to_string(dir.join(".markdown-import-hash"))
                    .await
                    .ok()
                    .as_deref()
                    == Some(&hash)
                {
                    continue;
                }
                apply_document(&mut r, document, true);
                s.save(&r).await.map_err(|e| e.to_string())?;
                tokio::fs::write(dir.join(".markdown-import-hash"), hash)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(Snapshot {
            sessions: s.list().await.map_err(|e| e.to_string())?,
            actions: s.globals().await.map_err(|e| e.1)?,
            theme: get_settings(Extract(s.clone()))
                .await
                .map_err(|e| e.1)?
                .0
                .theme,
        })
    }
    pub async fn output(s: &State, id: Uuid) -> String {
        match events(Extract(s.clone()), Path(id)).await {
            Ok(Json(value)) => format!(
                "{}\n{}",
                value["output"].as_str().unwrap_or(""),
                value["diagnostics"].as_str().unwrap_or("")
            ),
            Err(_) => String::new(),
        }
    }
    pub async fn execute(s: &State, command: Command) -> Result<Outcome, String> {
        let state = Extract(s.clone());
        let mut headers = HeaderMap::new();
        headers.insert("host", "127.0.0.1:8787".parse().unwrap());
        let mut outcome = Outcome {
            session: None,
            file: None,
            message: "Saved".into(),
        };
        match command {
            Command::GeminiSettings { settings, key } => {
                let _lock = s.jobs.lock().await;
                settings.validate().map_err(|e| e.to_string())?;
                crate::secrets::save(&s.root, &key).map_err(|e| e.to_string())?;
                settings.save(&s.root).map_err(|e| e.to_string())?;
                outcome.message = "Gemini settings saved".into();
            }
            Command::FinishLive { id, total, warning } => {
                crate::gemini::finish_live(s, id, total, warning)
                    .await
                    .map_err(|e| e.to_string())?;
                s.gemini.start(s.clone(), id).await;
                outcome.message =
                    "Recording saved. Gemini is finishing the remaining audio.".into();
            }
            Command::ProcessGemini(id) => {
                crate::gemini::prepare_import(s, id)
                    .await
                    .map_err(|e| e.to_string())?;
                s.gemini.start(s.clone(), id).await;
                outcome.message =
                    "Gemini is processing this recording in one-minute segments.".into();
            }
            Command::RetryGemini(id) => {
                crate::gemini::retry(s, id)
                    .await
                    .map_err(|e| e.to_string())?;
                outcome.message = "Retrying pending Gemini audio".into();
            }
            Command::Import {
                path,
                title,
                owned_capture,
            } => {
                let size = tokio::fs::metadata(&path)
                    .await
                    .map_err(|e| e.to_string())?
                    .len();
                if size == 0 || (!owned_capture && size > 150 * 1024 * 1024) {
                    return Err("Choose a nonempty file smaller than 150 MB".into());
                }
                if owned_capture {
                    let parent = path
                        .parent()
                        .ok_or("Invalid capture path")?
                        .canonicalize()
                        .map_err(|e| e.to_string())?;
                    if parent != s.root.canonicalize().map_err(|e| e.to_string())?
                        || !path
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with(".capture-"))
                    {
                        return Err("Capture file is outside this app's data directory".into());
                    }
                }
                let id = Uuid::new_v4();
                let dir = s.root.join(id.to_string());
                tokio::fs::create_dir(&dir)
                    .await
                    .map_err(|e| e.to_string())?;
                if owned_capture {
                    tokio::fs::rename(&path, dir.join("audio"))
                        .await
                        .map_err(|e| e.to_string())?;
                } else {
                    tokio::fs::copy(&path, dir.join("audio"))
                        .await
                        .map_err(|e| e.to_string())?;
                }
                let audio_type = match path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "wav" => "audio/wav",
                    "mp3" => "audio/mpeg",
                    "mp4" | "m4a" => "audio/mp4",
                    "ogg" | "opus" => "audio/ogg",
                    "flac" => "audio/flac",
                    "webm" => "audio/webm",
                    _ => "application/octet-stream",
                };
                let r = Recording {
                    id,
                    title: if title.trim().is_empty() {
                        "Untitled conversation".into()
                    } else {
                        title.chars().take(200).collect()
                    },
                    created_at: chrono::Utc::now().to_rfc3339(),
                    status: "recorded".into(),
                    transcript: String::new(),
                    gemini_processed: 0,
                    notes: String::new(),
                    error: None,
                    pinned: false,
                    archived_at: None,
                    note_actions: Default::default(),
                    audio_type: audio_type.into(),
                    actions: vec![],
                };
                if let Err(e) = s.save(&r).await {
                    if owned_capture {
                        let _ = tokio::fs::rename(dir.join("audio"), &path).await;
                    }
                    return Err(e.to_string());
                }
                outcome.session = Some(id);
                outcome.message = "Recording saved on this computer".into();
            }
            Command::ImportMarkdown { id, path } => {
                let _lock = s.jobs.lock().await;
                let mut r = s.load(id).await.map_err(|e| e.1)?;
                if r.status == "processing" || r.status == "recording" {
                    return Err("Stop recording before importing notes".into());
                }
                let size = tokio::fs::metadata(&path)
                    .await
                    .map_err(|e| e.to_string())?
                    .len();
                if size == 0 || size > 1_000_000 {
                    return Err("Choose a nonempty UTF-8 Markdown file under 1 MB".into());
                }
                let document = tokio::fs::read_to_string(path)
                    .await
                    .map_err(|e| format!("Cannot read UTF-8 Markdown: {e}"))?;
                apply_document(&mut r, document, true);
                s.save(&r).await.map_err(|e| e.to_string())?;
                outcome.session = Some(id);
                outcome.message = format!("Notes imported · {} action items", r.actions.len());
            }
            Command::Notes { id, notes } => {
                save_notes(
                    state,
                    Path(id),
                    headers,
                    Json(Notes {
                        notes: Some(notes),
                        pinned: None,
                    }),
                )
                .await
                .map_err(|e| e.1)?;
            }
            Command::Pin { id, pinned } => {
                save_notes(
                    state,
                    Path(id),
                    headers,
                    Json(Notes {
                        notes: None,
                        pinned: Some(pinned),
                    }),
                )
                .await
                .map_err(|e| e.1)?;
            }
            Command::Complete { id, action, done } => {
                session_toggle(state, Path((id, action)), Json(Toggle { done }))
                    .await
                    .map_err(|e| e.1)?;
            }
            Command::Copy { id, action } => {
                copy_action(
                    state,
                    Json(CopyAction {
                        session: id,
                        action,
                    }),
                )
                .await
                .map_err(|e| e.1)?;
                outcome.message = "Copied to global actions".into();
            }
            Command::GlobalComplete { id, done } => {
                global_toggle(state, Path(id), Json(Toggle { done }))
                    .await
                    .map_err(|e| e.1)?;
            }
            Command::ActionFromNote { id, title } => {
                let _lock = s.jobs.lock().await;
                let mut r = s.load(id).await.map_err(|e| e.1)?;
                let normalized = crate::document::action_title(&title);
                let point = crate::document::note_points(&r.notes)
                    .into_iter()
                    .find(|p| crate::document::action_title(&p.title) == normalized)
                    .ok_or("This point changed. Refresh the notes and try again.")?;
                let linked = r
                    .note_actions
                    .get(&normalized)
                    .is_some_and(|id| r.actions.iter().any(|a| a.id == *id));
                if !linked
                    && !r
                        .actions
                        .iter()
                        .any(|a| crate::document::action_title(&a.title) == normalized)
                {
                    r.notes = crate::document::append_action(&r.notes, &point.title);
                    let action_id = Uuid::new_v4();
                    r.note_actions.insert(normalized, action_id);
                    r.actions.push(Action {
                        id: action_id,
                        title: point.title,
                        owner: None,
                        due: None,
                        done: false,
                    });
                    s.save(&r).await.map_err(|e| e.to_string())?;
                    outcome.message = "Action item created from note".into();
                } else {
                    outcome.message = "This point already has an action item".into();
                }
            }
            Command::GlobalSave {
                id,
                title,
                owner,
                due,
            } => {
                let title = title.trim();
                let owner = owner.trim();
                let due = due.map(|d| d.trim().to_owned());
                if title.is_empty() || title.chars().count() > 2000 {
                    return Err("Enter a task title of 1–2,000 characters".into());
                }
                if owner.chars().count() > 200 {
                    return Err("Owner must be 200 characters or fewer".into());
                }
                if due
                    .as_ref()
                    .is_some_and(|d| !d.is_empty() && crate::tasks::parse_due(d).is_none())
                {
                    return Err("Choose a valid date in YYYY-MM-DD format".into());
                }
                let _lock = s.jobs.lock().await;
                let mut actions = s.globals().await.map_err(|e| e.1)?;

                let owner = (!owner.is_empty()).then(|| owner.to_owned());
                if let Some(id) = id {
                    let g = actions
                        .iter_mut()
                        .find(|g| g.id == id)
                        .ok_or("Task no longer exists")?;
                    // Editing content must preserve completion, pin and source identity.
                    g.action.title = title.into();
                    g.action.owner = owner;
                    if let Some(due) = due {
                        g.action.due = (!due.is_empty()).then_some(due);
                    }
                } else {
                    actions.push(GlobalAction {
                        id: Uuid::new_v4(),
                        source_session: Uuid::nil(),
                        source_action: Uuid::nil(),
                        source_title: "Personal task".into(),
                        action: Action {
                            id: Uuid::new_v4(),
                            title: title.into(),
                            owner,
                            due: due.filter(|d| !d.is_empty()),
                            done: false,
                        },
                        added_at: Some(chrono::Utc::now().to_rfc3339()),
                        pinned: false,
                    });
                }
                s.save_globals(&actions).await.map_err(|e| e.1)?;
                outcome.message = "Global task saved".into();
            }
            Command::GlobalPin { id, pinned } => {
                let _lock = s.jobs.lock().await;
                let mut actions = s.globals().await.map_err(|e| e.1)?;
                actions
                    .iter_mut()
                    .find(|g| g.id == id)
                    .ok_or("Task no longer exists")?
                    .pinned = pinned;
                s.save_globals(&actions).await.map_err(|e| e.1)?;
            }
            Command::GlobalRemove(id) => {
                global_delete(state, Path(id)).await.map_err(|e| e.1)?;
            }
            Command::Archive { id, archived } => {
                let _lock = s.jobs.lock().await;
                let mut r = s.load(id).await.map_err(|e| e.1)?;
                if r.status == "recording" || r.status == "processing" || s.gemini.running(id).await
                {
                    return Err(
                        "Wait until recording and processing finish before changing the archive"
                            .into(),
                    );
                }
                if archived {
                    // Repeated clicks must not extend the retention deadline.
                    if r.archived_at.is_none() {
                        r.archived_at = Some(chrono::Utc::now().to_rfc3339());
                    }
                    r.pinned = false;
                } else {
                    r.archived_at = None;
                }
                s.save(&r).await.map_err(|e| e.to_string())?;
                outcome.message = if archived {
                    "Archived; automatically deleted after 30 days"
                } else {
                    "Restored to library"
                }
                .into();
            }
            Command::Delete(id) => {
                delete(state, Path(id), headers).await.map_err(|e| e.1)?;
                outcome.message = "Session deleted; copied global actions retained".into();
            }
            Command::Theme(theme) => {
                set_settings(state, Json(Settings { theme }))
                    .await
                    .map_err(|e| e.1)?;
            }
            Command::Playback(id) => {
                s.load(id).await.map_err(|e| e.1)?;
                let dir = s.root.join(id.to_string());
                let wav = dir.join("audio");
                let reader = hound::WavReader::open(&wav).map_err(|_| "In-app playback supports PCM WAV recordings. Export other formats to play in your media app.".to_string())?;
                let spec = reader.spec();
                if spec.channels != 1
                    || spec.bits_per_sample != 16
                    || spec.sample_format != hound::SampleFormat::Int
                {
                    return Err("In-app playback requires mono 16-bit PCM WAV. Native recordings use this format.".into());
                }
                outcome.session = Some(id);
                outcome.file = Some(wav);
                outcome.message = "Playing audio".into();
            }
        }
        Ok(outcome)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[tokio::test]
        async fn native_commands_use_the_same_persistent_store_without_http() {
            let (s, r) = super::super::tests::fixture().await;
            execute(
                &s,
                Command::Pin {
                    id: r.id,
                    pinned: true,
                },
            )
            .await
            .unwrap();
            execute(
                &s,
                Command::Copy {
                    id: r.id,
                    action: r.actions[0].id,
                },
            )
            .await
            .unwrap();
            execute(
                &s,
                Command::Notes {
                    id: r.id,
                    notes: "## Native notes".into(),
                },
            )
            .await
            .unwrap();
            execute(&s, Command::Theme("dark".into())).await.unwrap();
            let data = snapshot(&s).await.unwrap();
            assert!(data.sessions[0].pinned);
            assert_eq!(data.actions.len(), 1);
            assert_eq!(data.theme, "dark");
            execute(&s, Command::Delete(r.id)).await.unwrap();
            assert_eq!(snapshot(&s).await.unwrap().actions.len(), 1);
            tokio::fs::remove_dir_all(s.root).await.unwrap();
        }
        #[tokio::test]
        async fn returned_markdown_import_watch_and_checklists_keep_bulgarian_and_identity() {
            let (s, r) = super::super::tests::fixture().await;
            let file = s.root.join("returned.md");
            let markdown = "# Среща\n\n## Actions\n- [ ] Мария да изпрати офертата\n- [x] Готов преглед\n\n## Full transcript\n**Мария:** Ще изпратя офертата.\n";
            tokio::fs::write(&file, markdown).await.unwrap();
            execute(
                &s,
                Command::ImportMarkdown {
                    id: r.id,
                    path: file,
                },
            )
            .await
            .unwrap();
            let imported = s.load(r.id).await.unwrap();
            assert_eq!(imported.actions.len(), 2);
            assert_eq!(imported.actions[0].title, "Мария да изпрати офертата");
            assert!(imported.actions[0].owner.is_none());
            assert!(imported.transcript.contains("Ще изпратя"));
            let id = imported.actions[0].id;
            execute(
                &s,
                Command::Copy {
                    id: r.id,
                    action: id,
                },
            )
            .await
            .unwrap();
            execute(
                &s,
                Command::Complete {
                    id: r.id,
                    action: id,
                    done: true,
                },
            )
            .await
            .unwrap();
            let dir = s.root.join(r.id.to_string());
            tokio::fs::write(dir.join("agent-output.md"), markdown)
                .await
                .unwrap();
            let first = snapshot(&s).await.unwrap();
            assert_eq!(first.sessions[0].actions[0].id, id);
            assert!(first.sessions[0].actions[0].done);
            assert!(!first.actions[0].action.done);
            // An unchanged return file must not overwrite a local checkbox edit.
            execute(
                &s,
                Command::Complete {
                    id: r.id,
                    action: id,
                    done: false,
                },
            )
            .await
            .unwrap();
            assert!(!snapshot(&s).await.unwrap().sessions[0].actions[0].done);
            let exported = tokio::fs::read_to_string(dir.join("notes.md"))
                .await
                .unwrap();
            assert_eq!(exported.matches("Мария да изпрати офертата").count(), 1);
            assert!(exported.contains("- [ ] Мария"));
            tokio::fs::write(
                dir.join("agent-output.md"),
                format!("{markdown}\n## More actions\n- [ ] Проверка на дизайна\n"),
            )
            .await
            .unwrap();
            assert_eq!(snapshot(&s).await.unwrap().sessions[0].actions.len(), 3);
            tokio::fs::remove_dir_all(s.root).await.unwrap();
        }
        #[tokio::test]
        async fn editing_markdown_preserves_existing_structured_actions() {
            let (s, r) = super::super::tests::fixture().await;
            let original = r.actions[0].id;
            execute(
                &s,
                Command::Notes {
                    id: r.id,
                    notes: "## Notes\n- [ ] New explicit task".into(),
                },
            )
            .await
            .unwrap();
            let saved = s.load(r.id).await.unwrap();
            assert_eq!(saved.actions.len(), 2);
            assert_eq!(saved.actions[0].id, original);
            assert_eq!(saved.actions[1].title, "New explicit task");
            tokio::fs::remove_dir_all(s.root).await.unwrap();
        }
        #[tokio::test]
        async fn owned_capture_is_moved_and_external_import_is_preserved() {
            let (s, _) = super::super::tests::fixture().await;
            let capture = s.root.join(".capture-test.wav");
            tokio::fs::write(&capture, b"recording").await.unwrap();
            let result = execute(
                &s,
                Command::Import {
                    path: capture.clone(),
                    title: "Native capture".into(),
                    owned_capture: true,
                },
            )
            .await
            .unwrap();
            assert!(!capture.exists());
            assert!(
                s.root
                    .join(result.session.unwrap().to_string())
                    .join("audio")
                    .is_file()
            );
            let external = s.root.join("meeting.wav");
            tokio::fs::write(&external, b"recording").await.unwrap();
            execute(
                &s,
                Command::Import {
                    path: external.clone(),
                    title: "Imported".into(),
                    owned_capture: false,
                },
            )
            .await
            .unwrap();
            assert!(external.exists());
            assert!(
                execute(
                    &s,
                    Command::Import {
                        path: external,
                        title: String::new(),
                        owned_capture: true
                    }
                )
                .await
                .is_err()
            );
            tokio::fs::remove_dir_all(s.root).await.unwrap();
        }
    }
}
pub(crate) fn default_data_root() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(exe) = std::env::current_exe()
            && let Some(parent) = exe.parent()
        {
            let legacy = parent.join("recordings");
            if legacy.is_dir() {
                return legacy;
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("OpenAIRecorder/recordings");
        }
    }
    PathBuf::from("recordings")
}
fn default_audio_type() -> String {
    "application/octet-stream".into()
}
fn internal(e: impl std::fmt::Display) -> Error {
    eprintln!("Local operation failed: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Local operation failed; inspect server logs".into(),
    )
}
impl State {
    pub async fn new() -> anyhow::Result<Self> {
        let initial = std::env::var("RECORDER_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_data_root());
        let root = crate::workspace::resolve(&initial, &crate::workspace::location_file())?;
        tokio::fs::create_dir_all(&root).await?;
        let root = tokio::fs::canonicalize(root).await?;
        let state = Self {
            root,
            overlay: Overlay::new(),
            jobs: Arc::new(Mutex::new(())),
            gemini: Arc::new(crate::gemini::Service::new().unwrap()),
        };
        // A process cannot survive an application restart. Make interrupted jobs retryable.
        for mut record in state.list().await? {
            if record.status == "processing" {
                record.status = "failed".into();
                record.error =
                    Some("Processing interrupted by server restart. You can retry.".into());
                state.save(&record).await?;
            }
        }
        state.prune_archives(chrono::Utc::now()).await?;
        crate::gemini::recover(&state).await?;
        Ok(state)
    }
    pub(crate) async fn prune_archives(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        let _lock = self.jobs.lock().await;
        for r in self.list().await? {
            let expired = r
                .archived_at
                .as_deref()
                .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
                .is_some_and(|date| now.signed_duration_since(date) >= chrono::Duration::days(30));
            if expired
                && r.status != "recording"
                && r.status != "processing"
                && !self.gemini.running(r.id).await
            {
                tokio::fs::remove_dir_all(self.root.join(r.id.to_string())).await?;
            }
        }
        Ok(())
    }
    pub(crate) async fn save(&self, r: &Recording) -> anyhow::Result<()> {
        let dir = self.root.join(r.id.to_string());
        tokio::fs::write(dir.join("record.json.tmp"), serde_json::to_vec_pretty(r)?).await?;
        tokio::fs::rename(dir.join("record.json.tmp"), dir.join("record.json")).await?;
        tokio::fs::write(
            dir.join("transcript.md"),
            format!("# {}\n\n{}\n", r.title, r.transcript),
        )
        .await?;
        let synced = crate::document::sync_tasks(&r.notes, |title| {
            r.actions.iter().find(|a| a.title == title).map(|a| a.done)
        });
        let actions = r
            .actions
            .iter()
            .map(|a| {
                format!(
                    "- [{}] {} (owner: {}; due: {})",
                    if a.done { "x" } else { " " },
                    a.title,
                    a.owner.as_deref().unwrap_or("unspecified"),
                    a.due.as_deref().unwrap_or("unspecified")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let exported = if crate::document::tasks(&r.notes).is_empty() {
            format!(
                "# {}\n\n{}\n\n## Action items\n{}\n",
                r.title, synced, actions
            )
        } else {
            synced
        };
        tokio::fs::write(dir.join("notes.md"), exported).await?;
        Ok(())
    }
    pub(crate) async fn load(&self, id: Uuid) -> Result<Recording, Error> {
        let bytes = tokio::fs::read(self.root.join(id.to_string()).join("record.json"))
            .await
            .map_err(|_| (StatusCode::NOT_FOUND, "Recording not found".into()))?;
        serde_json::from_slice(&bytes).map_err(internal)
    }
    pub(crate) async fn list(&self) -> anyhow::Result<Vec<Recording>> {
        let mut entries = tokio::fs::read_dir(&self.root).await?;
        let mut results = vec![];
        while let Some(entry) = entries.next_entry().await? {
            if let Ok(bytes) = tokio::fs::read(entry.path().join("record.json")).await {
                results.push(serde_json::from_slice::<Recording>(&bytes)?);
            }
        }
        results.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.created_at.cmp(&a.created_at))
        });
        Ok(results)
    }
}
pub fn router(state: State) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../web/index.html")) }),
        )
        .route("/api/settings", get(get_settings).put(set_settings))
        .route(
            "/api/capture",
            get(capture_status).post(capture_start).delete(capture_stop),
        )
        .route("/api/actions", get(global_list).post(copy_action))
        .route(
            "/api/actions/{id}",
            axum::routing::patch(global_toggle).delete(global_delete),
        )
        .route(
            "/api/recordings/{id}/actions/{action}",
            axum::routing::patch(session_toggle),
        )
        .route("/api/recordings/{id}/events", get(events))
        .route("/api/config", get(config))
        .route("/api/recordings", get(list).post(upload))
        .route(
            "/api/recordings/{id}",
            get(detail).delete(delete).put(save_notes),
        )
        .route("/api/recordings/{id}/audio", get(audio))
        .route("/api/recordings/{id}/process", post(process))
        .layer(DefaultBodyLimit::max(150 * 1024 * 1024))
        .layer(axum::middleware::from_fn(guard))
        .with_state(state)
}
async fn capture_start(Extract(s): Extract<State>) -> Result<StatusCode, Error> {
    if !s.overlay.start() {
        return Err((StatusCode::CONFLICT, "A recording is already active".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn capture_stop(Extract(s): Extract<State>) -> StatusCode {
    s.overlay.hide();
    StatusCode::NO_CONTENT
}
async fn capture_status(Extract(s): Extract<State>) -> Json<serde_json::Value> {
    use std::sync::atomic::Ordering;
    Json(
        serde_json::json!({"active":s.overlay.active.load(Ordering::SeqCst),"stop_requested":s.overlay.stop.swap(false,Ordering::SeqCst),"native_widget":cfg!(windows),"paused":s.overlay.pause.load(Ordering::SeqCst),"elapsed_ms":s.overlay.elapsed_ms.load(Ordering::Relaxed)}),
    )
}
async fn guard(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if let Err(e) = local(request.headers()) {
        return e.into_response();
    }
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("x-frame-options", "DENY".parse().unwrap());
    h.insert("cache-control", "no-store".parse().unwrap());
    h.insert("content-security-policy", "default-src 'self'; script-src 'self'; style-src 'self'; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'".parse().unwrap());
    response
}
// The server is loopback-only; reject cross-origin mutations and DNS rebinding.
fn local(headers: &HeaderMap) -> Result<(), Error> {
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !["127.0.0.1:8787", "localhost:8787"].contains(&host) {
        return Err((StatusCode::FORBIDDEN, "Use the local app address".into()));
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
    {
        return Err((StatusCode::FORBIDDEN, "Cross-site request rejected".into()));
    }
    if let Some(origin) = headers.get("origin")
        && origin.to_str().ok() != Some(format!("http://{host}").as_str())
    {
        return Err((
            StatusCode::FORBIDDEN,
            "Cross-origin request rejected".into(),
        ));
    }
    Ok(())
}
async fn config(Extract(s): Extract<State>) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"gemini_configured":crate::secrets::load(&s.root).is_ok_and(|key|!key.is_empty()),"gemini_models":crate::gemini::Settings::load(&s.root),"local_transcription":false}),
    )
}
async fn list(Extract(s): Extract<State>) -> Result<Json<Vec<Recording>>, Error> {
    Ok(Json(s.list().await.map_err(internal)?))
}
async fn detail(
    Extract(s): Extract<State>,
    Path(id): Path<Uuid>,
) -> Result<Json<Recording>, Error> {
    Ok(Json(s.load(id).await?))
}
async fn audio(
    Extract(s): Extract<State>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, Error> {
    s.load(id).await?;
    let bytes = tokio::fs::read(s.root.join(id.to_string()).join("audio"))
        .await
        .map_err(internal)?;
    let r = s.load(id).await?;
    let extension = match r.audio_type.as_str() {
        "audio/wav" => "wav",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "video/mp4" => "mp4",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "audio/webm" | "video/webm" => "webm",
        _ => "bin",
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        r.audio_type
            .parse()
            .unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
    );
    headers.insert(
        "content-disposition",
        format!("attachment; filename=\"conversation.{extension}\"")
            .parse()
            .unwrap(),
    );
    Ok((headers, bytes))
}
#[derive(Deserialize)]
struct Notes {
    notes: Option<String>,
    pinned: Option<bool>,
}
async fn save_notes(
    Extract(s): Extract<State>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Notes>,
) -> Result<StatusCode, Error> {
    local(&headers)?;
    if input.notes.as_ref().is_some_and(|n| n.len() > 1_000_000) {
        return Err((StatusCode::BAD_REQUEST, "Notes are too long".into()));
    }
    let _lock = s.jobs.lock().await;
    let mut r = s.load(id).await?;
    if r.status == "processing" || r.status == "recording" && input.notes.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "Stop live recording before editing its notes".into(),
        ));
    }
    if let Some(notes) = input.notes {
        if crate::document::tasks(&notes).is_empty() {
            r.notes = notes;
        } else {
            for task in crate::document::tasks(&notes) {
                if let Some(a) = r.actions.iter_mut().find(|a| a.title == task.title) {
                    a.done = task.done;
                } else {
                    r.actions.push(Action {
                        id: Uuid::new_v4(),
                        title: task.title,
                        owner: None,
                        due: None,
                        done: task.done,
                    });
                }
            }
            r.notes = notes;
        }
    }
    if let Some(pinned) = input.pinned {
        if pinned && r.archived_at.is_some() {
            return Err((
                StatusCode::CONFLICT,
                "Restore the session before pinning".into(),
            ));
        }
        r.pinned = pinned;
    }
    s.save(&r).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn upload(
    Extract(s): Extract<State>,
    headers: HeaderMap,
    mut body: Multipart,
) -> Result<Json<Recording>, Error> {
    local(&headers)?;
    let mut audio = None;
    let mut audio_type = default_audio_type();
    let mut title = "Untitled conversation".to_string();
    while let Some(field) = body.next_field().await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid upload (maximum 150 MB)".into(),
        )
    })? {
        match field.name() {
            Some("audio") => {
                let supplied_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim();
                if [
                    "audio/webm",
                    "audio/wav",
                    "audio/x-wav",
                    "audio/mpeg",
                    "audio/mp4",
                    "audio/ogg",
                    "audio/flac",
                    "video/mp4",
                    "video/webm",
                ]
                .contains(&supplied_type)
                {
                    audio_type = if supplied_type == "audio/x-wav" {
                        "audio/wav".into()
                    } else {
                        supplied_type.into()
                    };
                }
                let bytes = field.bytes().await.map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Cannot read audio (maximum 150 MB)".into(),
                    )
                })?;
                audio = Some(bytes);
            }
            Some("title") => {
                title = field
                    .text()
                    .await
                    .map_err(internal)?
                    .chars()
                    .take(200)
                    .collect();
            }
            _ => {}
        }
    }
    let audio = audio.filter(|b| !b.is_empty()).ok_or((
        StatusCode::BAD_REQUEST,
        "Choose a nonempty audio file".into(),
    ))?;
    let id = Uuid::new_v4();
    let dir = s.root.join(id.to_string());
    tokio::fs::create_dir(&dir).await.map_err(internal)?;
    tokio::fs::write(dir.join("audio"), audio)
        .await
        .map_err(internal)?;
    let r = Recording {
        id,
        title,
        created_at: chrono::Utc::now().to_rfc3339(),
        status: "recorded".into(),
        transcript: String::new(),
        gemini_processed: 0,
        notes: String::new(),
        error: None,
        pinned: false,
        archived_at: None,
        note_actions: Default::default(),
        audio_type,
        actions: vec![],
    };
    s.save(&r).await.map_err(internal)?;
    Ok(Json(r))
}
async fn process(
    Extract(s): Extract<State>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, Error> {
    local(&headers)?;
    crate::gemini::prepare_import(&s, id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    s.gemini.start(s.clone(), id).await;
    Ok(StatusCode::ACCEPTED)
}
async fn events(
    Extract(s): Extract<State>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, Error> {
    s.load(id).await?;
    let dir = s.root.join(id.to_string());
    let output = tokio::fs::read_to_string(dir.join("agent-events.jsonl"))
        .await
        .unwrap_or_default();
    let diagnostics = tokio::fs::read_to_string(dir.join("agent-errors.log"))
        .await
        .unwrap_or_default();
    Ok(Json(
        serde_json::json!({"output":output,"diagnostics":diagnostics}),
    ))
}
#[derive(Deserialize)]
struct Toggle {
    done: bool,
}
async fn session_toggle(
    Extract(s): Extract<State>,
    Path((id, action)): Path<(Uuid, Uuid)>,
    Json(input): Json<Toggle>,
) -> Result<StatusCode, Error> {
    let _lock = s.jobs.lock().await;
    let mut r = s.load(id).await?;
    if r.status == "processing" {
        return Err((StatusCode::CONFLICT, "Wait for processing to finish".into()));
    }
    r.actions
        .iter_mut()
        .find(|a| a.id == action)
        .ok_or((StatusCode::NOT_FOUND, "Action not found".into()))?
        .done = input.done;
    r.notes = crate::document::sync_tasks(&r.notes, |title| {
        r.actions.iter().find(|a| a.title == title).map(|a| a.done)
    });
    s.save(&r).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
impl State {
    async fn globals(&self) -> Result<Vec<GlobalAction>, Error> {
        match tokio::fs::read(self.root.join("global-actions.json")).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(internal),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(internal(e)),
        }
    }
    async fn save_globals(&self, actions: &[GlobalAction]) -> Result<(), Error> {
        tokio::fs::write(
            self.root.join("global-actions.json.tmp"),
            serde_json::to_vec_pretty(actions).map_err(internal)?,
        )
        .await
        .map_err(internal)?;
        tokio::fs::rename(
            self.root.join("global-actions.json.tmp"),
            self.root.join("global-actions.json"),
        )
        .await
        .map_err(internal)?;
        Ok(())
    }
}
async fn global_list(Extract(s): Extract<State>) -> Result<Json<Vec<GlobalAction>>, Error> {
    Ok(Json(s.globals().await?))
}
#[derive(Deserialize)]
struct CopyAction {
    session: Uuid,
    action: Uuid,
}
async fn copy_action(
    Extract(s): Extract<State>,
    Json(input): Json<CopyAction>,
) -> Result<StatusCode, Error> {
    let _lock = s.jobs.lock().await;
    let r = s.load(input.session).await?;
    let action = r
        .actions
        .iter()
        .find(|a| a.id == input.action)
        .ok_or((StatusCode::NOT_FOUND, "Action not found".into()))?
        .clone();
    let mut globals = s.globals().await?;
    if !globals
        .iter()
        .any(|g| g.source_session == input.session && g.source_action == input.action)
    {
        globals.push(GlobalAction {
            id: Uuid::new_v4(),
            source_session: input.session,
            source_title: r.title,
            source_action: input.action,
            added_at: Some(chrono::Utc::now().to_rfc3339()),
            pinned: false,
            action,
        });
        s.save_globals(&globals).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn global_toggle(
    Extract(s): Extract<State>,
    Path(id): Path<Uuid>,
    Json(input): Json<Toggle>,
) -> Result<StatusCode, Error> {
    let _lock = s.jobs.lock().await;
    let mut actions = s.globals().await?;
    actions
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or((StatusCode::NOT_FOUND, "Action not found".into()))?
        .action
        .done = input.done;
    s.save_globals(&actions).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn global_delete(
    Extract(s): Extract<State>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    let _lock = s.jobs.lock().await;
    let mut actions = s.globals().await?;
    actions.retain(|a| a.id != id);
    s.save_globals(&actions).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete(
    Extract(s): Extract<State>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, Error> {
    local(&headers)?;
    if s.gemini.running(id).await {
        return Err((
            StatusCode::CONFLICT,
            "Wait for Gemini processing to finish before deleting".into(),
        ));
    }
    let _lock = s.jobs.lock().await;
    let r = s.load(id).await?;
    if r.status == "processing" || r.status == "recording" {
        return Err((
            StatusCode::CONFLICT,
            "Wait for processing to finish before deleting".into(),
        ));
    }
    tokio::fs::remove_dir_all(s.root.join(id.to_string()))
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    #[tokio::test]
    async fn upload_persists_and_delete_removes_recording() {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        tokio::fs::create_dir(&root).await.unwrap();
        let state = State {
            root: root.clone(),
            overlay: Overlay::new(),
            jobs: Arc::new(Mutex::new(())),
            gemini: Arc::new(crate::gemini::Service::new().unwrap()),
        };
        let app = router(state.clone());
        let payload = "--test\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nMeeting\r\n--test\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\naudio-data\r\n--test--\r\n";
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/recordings")
                    .header("host", "127.0.0.1:8787")
                    .header("content-type", "multipart/form-data; boundary=test")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let records = state.list().await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "Meeting");
        assert_eq!(
            tokio::fs::read(root.join(records[0].id.to_string()).join("audio"))
                .await
                .unwrap(),
            b"audio-data"
        );
        let response = app
            .oneshot(
                Request::delete(format!("/api/recordings/{}", records[0].id))
                    .header("host", "127.0.0.1:8787")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(state.list().await.unwrap().is_empty());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
    #[test]
    fn cross_origin_and_rebinding_are_rejected() {
        let mut h = HeaderMap::new();
        h.insert("host", "127.0.0.1:8787".parse().unwrap());
        assert!(local(&h).is_ok());
        h.insert("origin", "https://evil.example".parse().unwrap());
        assert!(local(&h).is_err());
        h.remove("origin");
        h.insert("host", "evil.example:8787".parse().unwrap());
        assert!(local(&h).is_err());
    }
    pub(super) async fn fixture() -> (State, Recording) {
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        let id = Uuid::new_v4();
        tokio::fs::create_dir_all(root.join(id.to_string()))
            .await
            .unwrap();
        let s = State {
            root,
            overlay: Overlay::new(),
            jobs: Arc::new(Mutex::new(())),
            gemini: Arc::new(crate::gemini::Service::new().unwrap()),
        };
        let r = Recording {
            id,
            title: "Planning".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            status: "transcribed".into(),
            transcript: "Maya will send the proposal Friday.".into(),
            gemini_processed: 0,
            notes: "# Summary\nProposal agreed.".into(),
            error: None,
            pinned: false,
            archived_at: None,
            note_actions: Default::default(),
            audio_type: "audio/wav".into(),
            actions: vec![Action {
                id: Uuid::new_v4(),
                title: "Send proposal".into(),
                owner: Some("Maya".into()),
                due: Some("Friday".into()),
                done: false,
            }],
        };
        s.save(&r).await.unwrap();
        (s, r)
    }
    #[tokio::test]
    async fn note_points_create_persistent_actions_once_and_keep_original_notes_and_global_copy() {
        use desktop_api::{Command, execute};
        let (s, mut r) = fixture().await;
        r.notes="## Key Points\n- **Изпрати** офертата.\n## Decisions\n- Release next week.\n## Open Questions\n- Who owns testing?\n".into();
        s.save(&r).await.unwrap();
        for _ in 0..2 {
            execute(
                &s,
                Command::ActionFromNote {
                    id: r.id,
                    title: "Изпрати офертата.".into(),
                },
            )
            .await
            .unwrap();
        }
        let saved = s.load(r.id).await.unwrap();
        assert_eq!(saved.actions.len(), 2);
        let action = saved
            .actions
            .iter()
            .find(|a| a.title == "Изпрати офертата.")
            .unwrap();
        assert!(!action.done && action.owner.is_none() && action.due.is_none());
        assert!(saved.notes.starts_with(&r.notes));
        assert_eq!(crate::document::tasks(&saved.notes).len(), 1);
        execute(
            &s,
            Command::Complete {
                id: r.id,
                action: action.id,
                done: true,
            },
        )
        .await
        .unwrap();
        execute(
            &s,
            Command::ActionFromNote {
                id: r.id,
                title: "изпрати ОФЕРТАТА.".into(),
            },
        )
        .await
        .unwrap();
        let checked = s.load(r.id).await.unwrap();
        assert!(
            checked
                .actions
                .iter()
                .find(|a| a.id == action.id)
                .unwrap()
                .done
        );
        execute(
            &s,
            Command::Copy {
                id: r.id,
                action: action.id,
            },
        )
        .await
        .unwrap();
        assert_eq!(s.globals().await.unwrap()[0].source_action, action.id);
        let mut rewritten = s.load(r.id).await.unwrap();
        rewritten
            .actions
            .iter_mut()
            .find(|a| a.id == action.id)
            .unwrap()
            .title = "AI-refined task wording".into();
        s.save(&rewritten).await.unwrap();
        execute(
            &s,
            Command::ActionFromNote {
                id: r.id,
                title: "Изпрати офертата.".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(s.load(r.id).await.unwrap().actions.len(), 2);

        assert!(
            execute(
                &s,
                Command::ActionFromNote {
                    id: r.id,
                    title: "Not in the notes".into()
                }
            )
            .await
            .is_err()
        );
        let disk = tokio::fs::read_to_string(s.root.join(r.id.to_string()).join("notes.md"))
            .await
            .unwrap();
        assert!(disk.contains("[x]"));
        assert_eq!(s.load(r.id).await.unwrap().actions.len(), 2);
        tokio::fs::remove_dir_all(s.root).await.unwrap();
    }
    #[tokio::test]
    async fn personal_tasks_validate_persist_and_edit_without_losing_completion_or_pin() {
        use desktop_api::{Command, execute};
        let (s, _) = fixture().await;
        execute(
            &s,
            Command::GlobalSave {
                id: None,
                title: "  Моя задача  ".into(),
                owner: " Мария ".into(),
                due: Some("2026-10-20".into()),
            },
        )
        .await
        .unwrap();
        let g = s.globals().await.unwrap().remove(0);
        assert!(g.source_session.is_nil());
        assert_eq!(g.action.title, "Моя задача");
        assert_eq!(g.action.owner.as_deref(), Some("Мария"));
        assert!(chrono::DateTime::parse_from_rfc3339(g.added_at.as_ref().unwrap()).is_ok());
        let (pin, complete, edit) = tokio::join!(
            execute(
                &s,
                Command::GlobalPin {
                    id: g.id,
                    pinned: true
                }
            ),
            execute(
                &s,
                Command::GlobalComplete {
                    id: g.id,
                    done: true
                }
            ),
            execute(
                &s,
                Command::GlobalSave {
                    id: Some(g.id),
                    title: "Edited".into(),
                    owner: String::new(),
                    due: Some("2026-11-01".into())
                }
            )
        );
        pin.unwrap();
        complete.unwrap();
        edit.unwrap();
        let edited = s.globals().await.unwrap().remove(0);
        assert!(edited.pinned && edited.action.done);
        assert_eq!(edited.action.title, "Edited");
        assert_eq!(edited.action.due.as_deref(), Some("2026-11-01"));
        assert!(edited.action.owner.is_none());
        assert_eq!(edited.added_at, g.added_at);
        assert_eq!(edited.action.id, g.action.id);
        for (title, due) in [("", ""), ("Valid", "2026-02-29"), ("Valid", "Friday")] {
            assert!(
                execute(
                    &s,
                    Command::GlobalSave {
                        id: Some(g.id),
                        title: title.into(),
                        owner: String::new(),
                        due: Some(due.into())
                    }
                )
                .await
                .is_err()
            );
        }
        assert_eq!(s.globals().await.unwrap()[0].action.title, "Edited");
        execute(
            &s,
            Command::GlobalSave {
                id: Some(g.id),
                title: "No deadline".into(),
                owner: String::new(),
                due: Some(String::new()),
            },
        )
        .await
        .unwrap();
        assert!(s.globals().await.unwrap()[0].action.due.is_none());
        // Exercise a fresh store read, rather than relying on GUI state.
        let disk: Vec<GlobalAction> = serde_json::from_slice(
            &tokio::fs::read(s.root.join("global-actions.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(disk[0].pinned && disk[0].action.done);
        tokio::fs::remove_dir_all(s.root).await.unwrap();
    }
    #[tokio::test]
    async fn editing_legacy_global_copies_preserves_source_freeform_due_and_copy_identity() {
        use desktop_api::{Command, execute};
        let (s, r) = fixture().await;
        copy_action(
            Extract(s.clone()),
            Json(CopyAction {
                session: r.id,
                action: r.actions[0].id,
            }),
        )
        .await
        .unwrap();
        let g = s.globals().await.unwrap().remove(0);
        let mut old = serde_json::to_value(vec![g.clone()]).unwrap();
        old[0].as_object_mut().unwrap().remove("added_at");
        old[0].as_object_mut().unwrap().remove("pinned");
        tokio::fs::write(
            s.root.join("global-actions.json"),
            serde_json::to_vec(&old).unwrap(),
        )
        .await
        .unwrap();
        execute(
            &s,
            Command::GlobalSave {
                id: Some(g.id),
                title: "Independently edited".into(),
                owner: String::new(),
                due: None,
            },
        )
        .await
        .unwrap();
        let edited = s.globals().await.unwrap().remove(0);
        assert_eq!(edited.action.due.as_deref(), Some("Friday"));
        assert!(edited.added_at.is_none());
        assert!(!edited.pinned);
        assert_eq!(edited.source_session, r.id);
        assert_eq!(edited.source_action, r.actions[0].id);
        assert_eq!(
            s.load(r.id).await.unwrap().actions[0].title,
            r.actions[0].title
        );
        copy_action(
            Extract(s.clone()),
            Json(CopyAction {
                session: r.id,
                action: r.actions[0].id,
            }),
        )
        .await
        .unwrap();
        assert_eq!(s.globals().await.unwrap().len(), 1);
        assert_eq!(
            s.globals().await.unwrap()[0].action.title,
            "Independently edited"
        );
        tokio::fs::remove_dir_all(s.root).await.unwrap();
    }
    #[tokio::test]
    async fn archive_retention_uses_archive_date_restores_and_preserves_global_actions() {
        use desktop_api::{Command, execute};
        let (s, mut r) = fixture().await;
        r.created_at = "2000-01-01T00:00:00Z".into();
        r.pinned = true;
        s.save(&r).await.unwrap();
        copy_action(
            Extract(s.clone()),
            Json(CopyAction {
                session: r.id,
                action: r.actions[0].id,
            }),
        )
        .await
        .unwrap();
        execute(
            &s,
            Command::Archive {
                id: r.id,
                archived: true,
            },
        )
        .await
        .unwrap();
        let archived = s.load(r.id).await.unwrap();
        assert!(!archived.pinned);
        let date = archived.archived_at.clone().unwrap();
        execute(
            &s,
            Command::Archive {
                id: r.id,
                archived: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            s.load(r.id).await.unwrap().archived_at.as_deref(),
            Some(date.as_str())
        );
        assert!(
            execute(
                &s,
                Command::Pin {
                    id: r.id,
                    pinned: true
                }
            )
            .await
            .is_err()
        );
        let start = chrono::DateTime::parse_from_rfc3339(&date)
            .unwrap()
            .with_timezone(&chrono::Utc);
        s.prune_archives(start + chrono::Duration::days(30) - chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert!(s.root.join(r.id.to_string()).exists());
        execute(
            &s,
            Command::Archive {
                id: r.id,
                archived: false,
            },
        )
        .await
        .unwrap();
        assert!(s.load(r.id).await.unwrap().archived_at.is_none());
        s.prune_archives(start + chrono::Duration::days(31))
            .await
            .unwrap();
        assert!(s.root.join(r.id.to_string()).exists());
        // Expiration removes all audio/notes/segments, without touching global copies.
        r.archived_at = Some(date);
        r.pinned = false;
        s.save(&r).await.unwrap();
        tokio::fs::write(
            s.root.join(r.id.to_string()).join("audio"),
            b"fixture audio",
        )
        .await
        .unwrap();
        s.prune_archives(start + chrono::Duration::days(30))
            .await
            .unwrap();
        assert!(!s.root.join(r.id.to_string()).exists());
        assert_eq!(s.globals().await.unwrap().len(), 1);
        tokio::fs::remove_dir_all(s.root).await.unwrap();
    }
    #[tokio::test]
    async fn archives_protect_active_sessions_and_legacy_metadata() {
        use desktop_api::{Command, execute};
        let (s, mut r) = fixture().await;
        let mut legacy = serde_json::to_value(&r).unwrap();
        legacy.as_object_mut().unwrap().remove("archived_at");
        assert!(
            serde_json::from_value::<Recording>(legacy)
                .unwrap()
                .archived_at
                .is_none()
        );
        r.status = "recording".into();
        s.save(&r).await.unwrap();
        assert!(
            execute(
                &s,
                Command::Archive {
                    id: r.id,
                    archived: true
                }
            )
            .await
            .is_err()
        );
        r.archived_at = Some("2000-01-01T00:00:00Z".into());
        s.save(&r).await.unwrap();
        s.prune_archives(chrono::Utc::now()).await.unwrap();
        assert!(s.root.join(r.id.to_string()).exists());
        r.status = "ready".into();
        r.archived_at = Some("bad timestamp".into());
        s.save(&r).await.unwrap();
        s.prune_archives(chrono::Utc::now()).await.unwrap();
        assert!(s.root.join(r.id.to_string()).exists());
        r.archived_at = Some((chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339());
        s.save(&r).await.unwrap();
        s.prune_archives(chrono::Utc::now()).await.unwrap();
        assert!(s.root.join(r.id.to_string()).exists());
        tokio::fs::remove_dir_all(s.root).await.unwrap();
    }
    #[tokio::test]
    async fn global_actions_survive_session_deletion_and_completion_is_independent() {
        let (s, r) = fixture().await;
        let action = r.actions[0].id;
        copy_action(
            Extract(s.clone()),
            Json(CopyAction {
                session: r.id,
                action,
            }),
        )
        .await
        .unwrap();
        copy_action(
            Extract(s.clone()),
            Json(CopyAction {
                session: r.id,
                action,
            }),
        )
        .await
        .unwrap();
        let globals = s.globals().await.unwrap();
        assert_eq!(globals.len(), 1);
        global_toggle(
            Extract(s.clone()),
            Path(globals[0].id),
            Json(Toggle { done: true }),
        )
        .await
        .unwrap();
        assert!(!s.load(r.id).await.unwrap().actions[0].done);
        let mut headers = HeaderMap::new();
        headers.insert("host", "localhost:8787".parse().unwrap());
        delete(Extract(s.clone()), Path(r.id), headers)
            .await
            .unwrap();
        let globals = s.globals().await.unwrap();
        assert!(globals[0].action.done);
        assert_eq!(globals[0].source_title, "Planning");
        tokio::fs::remove_dir_all(s.root).await.unwrap();
    }
    #[tokio::test]
    async fn pinned_sessions_sort_first_and_checked_actions_export_to_markdown() {
        let (s, mut r) = fixture().await;
        r.pinned = true;
        r.created_at = "2000-01-01T00:00:00Z".into();
        s.save(&r).await.unwrap();
        let mut newer = r.clone();
        newer.id = Uuid::new_v4();
        newer.pinned = false;
        newer.created_at = "2026-10-14T00:00:00Z".into();
        tokio::fs::create_dir(s.root.join(newer.id.to_string()))
            .await
            .unwrap();
        s.save(&newer).await.unwrap();
        assert_eq!(s.list().await.unwrap()[0].id, r.id);
        session_toggle(
            Extract(s.clone()),
            Path((r.id, r.actions[0].id)),
            Json(Toggle { done: true }),
        )
        .await
        .unwrap();
        let md = tokio::fs::read_to_string(s.root.join(r.id.to_string()).join("notes.md"))
            .await
            .unwrap();
        assert!(md.contains("- [x] Send proposal"));
        tokio::fs::remove_dir_all(s.root).await.unwrap();
    }
    #[tokio::test]
    async fn processing_blocks_delete_and_edit_and_capture_can_stop() {
        let (s, mut r) = fixture().await;
        r.status = "processing".into();
        s.save(&r).await.unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("host", "localhost:8787".parse().unwrap());
        assert_eq!(
            delete(Extract(s.clone()), Path(r.id), headers.clone())
                .await
                .unwrap_err()
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            save_notes(
                Extract(s.clone()),
                Path(r.id),
                headers,
                Json(Notes {
                    notes: Some("new".into()),
                    pinned: None
                })
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::CONFLICT
        );
        assert!(s.overlay.start());
        assert!(!s.overlay.start());
        s.overlay
            .stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            capture_status(Extract(s.clone())).await.0["stop_requested"],
            true
        );
        assert_eq!(
            capture_status(Extract(s.clone())).await.0["stop_requested"],
            false
        );
        s.overlay.hide();
        assert!(s.overlay.start());
        s.overlay.hide();
        tokio::fs::remove_dir_all(s.root).await.unwrap();
    }
    #[tokio::test]
    async fn theme_preference_persists_and_invalid_theme_is_rejected() {
        let (s, _) = fixture().await;
        assert_eq!(
            get_settings(Extract(s.clone())).await.unwrap().0.theme,
            "system"
        );
        set_settings(
            Extract(s.clone()),
            Json(Settings {
                theme: "dark".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            get_settings(Extract(s.clone())).await.unwrap().0.theme,
            "dark"
        );
        assert_eq!(
            set_settings(
                Extract(s.clone()),
                Json(Settings {
                    theme: "unknown".into()
                })
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            get_settings(Extract(s.clone())).await.unwrap().0.theme,
            "dark"
        );
        tokio::fs::remove_dir_all(s.root).await.unwrap();
    }
}
