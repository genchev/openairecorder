use crate::{agent::Agents, overlay::Overlay, transcription::Transcriber};
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
    pub transcriber: Transcriber,
    pub agents: Agents,
    pub overlay: Overlay,
    pub jobs: Arc<Mutex<()>>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Recording {
    pub id: Uuid,
    pub title: String,
    pub created_at: String,
    pub status: String,
    pub transcript: String,
    pub notes: String,
    pub error: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default = "default_audio_type")]
    pub audio_type: String,
    #[serde(default)]
    pub actions: Vec<Action>,
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
        Import {
            path: PathBuf,
            title: String,
            owned_capture: bool,
        },
        Transcribe(Uuid),
        Generate {
            id: Uuid,
            provider: String,
            consent: bool,
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
        GlobalRemove(Uuid),
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
                    notes: String::new(),
                    error: None,
                    pinned: false,
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
            Command::Transcribe(id) => {
                process(state, Path(id), headers).await.map_err(|e| e.1)?;
                outcome.message = "Transcribing locally…".into();
            }
            Command::Generate {
                id,
                provider,
                consent,
            } => {
                generate_notes(state, Path(id), Json(AgentRequest { provider, consent }))
                    .await
                    .map_err(|e| e.1)?;
                outcome.message = "Agent is generating notes…".into();
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
            Command::GlobalRemove(id) => {
                global_delete(state, Path(id)).await.map_err(|e| e.1)?;
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
                let wav = dir.join("playback.wav");
                if !wav.is_file() {
                    let mut cmd = tokio::process::Command::new(&s.transcriber.ffmpeg);
                    #[cfg(windows)]
                    cmd.creation_flags(0x08000000);
                    let result = cmd
                        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
                        .arg(dir.join("audio"))
                        .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
                        .arg(&wav)
                        .output()
                        .await
                        .map_err(|e| format!("Cannot start ffmpeg: {e}"))?;
                    if !result.status.success() {
                        let _ = tokio::fs::remove_file(wav).await;
                        return Err("Cannot decode this audio file for playback".into());
                    }
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
fn default_data_root() -> PathBuf {
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
        let root = std::env::var("RECORDER_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_data_root());
        tokio::fs::create_dir_all(&root).await?;
        let state = Self {
            root,
            transcriber: Transcriber::from_env(),
            agents: Agents::from_env(),
            overlay: Overlay::new(),
            jobs: Arc::new(Mutex::new(())),
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
        Ok(state)
    }
    async fn save(&self, r: &Recording) -> anyhow::Result<()> {
        let dir = self.root.join(r.id.to_string());
        tokio::fs::write(dir.join("record.json.tmp"), serde_json::to_vec_pretty(r)?).await?;
        tokio::fs::rename(dir.join("record.json.tmp"), dir.join("record.json")).await?;
        tokio::fs::write(
            dir.join("transcript.md"),
            format!("# {}\n\n{}\n", r.title, r.transcript),
        )
        .await?;
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
        tokio::fs::write(
            dir.join("notes.md"),
            format!(
                "# {}\n\n{}\n\n## Action items\n{}\n",
                r.title, r.notes, actions
            ),
        )
        .await?;
        Ok(())
    }
    async fn load(&self, id: Uuid) -> Result<Recording, Error> {
        let bytes = tokio::fs::read(self.root.join(id.to_string()).join("record.json"))
            .await
            .map_err(|_| (StatusCode::NOT_FOUND, "Recording not found".into()))?;
        serde_json::from_slice(&bytes).map_err(internal)
    }
    async fn list(&self) -> anyhow::Result<Vec<Recording>> {
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
        .route(
            "/app.js",
            get(|| async {
                (
                    [("content-type", "application/javascript")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [("content-type", "text/css")],
                    include_str!("../web/style.css"),
                )
            }),
        )
        .route(
            "/theme.js",
            get(|| async {
                (
                    [("content-type", "application/javascript")],
                    include_str!("../web/theme.js"),
                )
            }),
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
        .route("/api/recordings/{id}/agent", post(generate_notes))
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
        serde_json::json!({"local_transcription":s.transcriber.available().await,"codex":s.agents.detect("codex").await,"claude":s.agents.detect("claude").await}),
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
    if r.status == "processing" {
        return Err((
            StatusCode::CONFLICT,
            "Wait for transcription to finish".into(),
        ));
    }
    if let Some(notes) = input.notes {
        r.notes = notes;
    }
    if let Some(pinned) = input.pinned {
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
        notes: String::new(),
        error: None,
        pinned: false,
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
    let _lock = s.jobs.lock().await;
    let mut r = s.load(id).await?;
    if r.status == "processing" {
        return Err((StatusCode::CONFLICT, "Already processing".into()));
    }
    r.status = "processing".into();
    r.error = None;
    s.save(&r).await.map_err(internal)?;
    drop(_lock);
    tokio::spawn(async move {
        let result = run_job(&s, &mut r).await;
        match result {
            Ok(()) => r.status = "ready".into(),
            Err(e) => {
                r.status = "failed".into();
                r.error = Some(e.to_string());
            }
        }
        if let Err(e) = s.save(&r).await {
            eprintln!("Unable to save job result: {e}");
        }
    });
    Ok(StatusCode::ACCEPTED)
}
async fn run_job(s: &State, r: &mut Recording) -> anyhow::Result<()> {
    let dir = s.root.join(r.id.to_string());
    r.transcript = s.transcriber.transcribe(&dir.join("audio"), &dir).await?;
    Ok(())
}
#[derive(Deserialize)]
struct AgentRequest {
    provider: String,
    consent: bool,
}
async fn generate_notes(
    Extract(s): Extract<State>,
    Path(id): Path<Uuid>,
    Json(input): Json<AgentRequest>,
) -> Result<StatusCode, Error> {
    if !input.consent {
        return Err((
            StatusCode::BAD_REQUEST,
            "Confirm sharing the transcript with your selected agent".into(),
        ));
    }
    if !["codex", "claude"].contains(&input.provider.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "Unknown agent".into()));
    }
    if !s.agents.detect(&input.provider).await.installed {
        return Err((
            StatusCode::BAD_REQUEST,
            "Agent not installed. Follow the installation instructions in the app.".into(),
        ));
    }
    let _lock = s.jobs.lock().await;
    let mut r = s.load(id).await?;
    if r.status == "processing" {
        return Err((
            StatusCode::CONFLICT,
            "A job is already running for this session".into(),
        ));
    }
    if r.transcript.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Generate a local transcript first".into(),
        ));
    }
    r.status = "processing".into();
    r.error = None;
    let dir = s
        .root
        .join(id.to_string())
        .canonicalize()
        .map_err(internal)?;
    tokio::fs::write(dir.join("agent-events.jsonl"), b"")
        .await
        .map_err(internal)?;
    tokio::fs::write(dir.join("agent-errors.log"), b"")
        .await
        .map_err(internal)?;
    s.save(&r).await.map_err(internal)?;
    drop(_lock);
    tokio::spawn(async move {
        match s
            .agents
            .generate(&input.provider, &r.transcript, &dir)
            .await
        {
            Ok(output) => {
                r.notes = output.notes_markdown;
                r.actions = output
                    .actions
                    .into_iter()
                    .map(|a| {
                        let old = r.actions.iter().find(|old| old.title == a.title);
                        Action {
                            id: old.map(|o| o.id).unwrap_or_else(Uuid::new_v4),
                            title: a.title,
                            owner: a.owner,
                            due: a.due,
                            done: old.is_some_and(|o| o.done),
                        }
                    })
                    .collect();
                r.status = "ready".into();
            }
            Err(e) => {
                r.status = "failed".into();
                r.error = Some(e.to_string());
            }
        }
        if let Err(e) = s.save(&r).await {
            eprintln!("Cannot save notes: {e}");
        }
    });
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
    let _lock = s.jobs.lock().await;
    let r = s.load(id).await?;
    if r.status == "processing" {
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
            transcriber: Transcriber::from_env(),
            agents: Agents::from_env(),
            overlay: Overlay::new(),
            jobs: Arc::new(Mutex::new(())),
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
            transcriber: Transcriber::from_env(),
            agents: Agents::from_env(),
            overlay: Overlay::new(),
            jobs: Arc::new(Mutex::new(())),
        };
        let r = Recording {
            id,
            title: "Planning".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            status: "transcribed".into(),
            transcript: "Maya will send the proposal Friday.".into(),
            notes: "# Summary\nProposal agreed.".into(),
            error: None,
            pinned: false,
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
