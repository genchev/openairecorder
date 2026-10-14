use crate::{
    agent::Availability,
    app::{
        self,
        desktop_api::{self, Command, Outcome, Snapshot},
    },
};
use eframe::egui;
use std::{
    collections::VecDeque,
    sync::mpsc,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq)]
enum View {
    New,
    Session(Uuid),
    Actions,
    Settings,
}
#[derive(Default)]
struct Tools {
    speech: bool,
    codex: Availability,
    claude: Availability,
    checked: bool,
}
enum Message {
    Snapshot(u64, Result<Snapshot, String>, Option<(Uuid, String)>),
    Tools(Tools),
    Finished(Result<Outcome, String>),
    #[cfg_attr(not(windows), allow(dead_code))]
    Notice(String),
}
pub struct Desktop {
    state: app::State,
    runtime: tokio::runtime::Handle,
    sender: mpsc::Sender<Message>,
    receiver: mpsc::Receiver<Message>,
    data: Snapshot,
    tools: Tools,
    view: View,
    search: String,
    title: String,
    system_audio: bool,
    mic: bool,
    permission: bool,
    provider: String,
    ai_consent: bool,
    notes_edit: String,
    notes_dirty: bool,
    output: String,
    notice: String,
    theme: String,
    hide_completed: bool,
    busy: bool,
    epoch: u64,
    queued: VecDeque<Command>,
    notes_save_pending: bool,
    #[cfg(not(windows))]
    import_path: String,
    refreshing: bool,
    last_refresh: Instant,
    pending_navigation: Option<View>,
    delete_confirm: Option<Uuid>,
    close_confirm: bool,
    closing: bool,
    #[cfg(windows)]
    capture: Option<crate::capture::Capture>,
    #[cfg(windows)]
    recording_title: String,
    #[cfg(windows)]
    recording_warning: String,
    #[cfg(windows)]
    player: Option<crate::playback::Player>,
}
pub fn run(state: app::State, runtime: tokio::runtime::Handle) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 850.0])
            .with_min_inner_size([760.0, 550.0])
            .with_title("OpenAIRecorder"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "OpenAIRecorder",
        options,
        Box::new(move |cc| Ok(Box::new(Desktop::new(state, runtime, &cc.egui_ctx)))),
    )
    .map_err(|e| anyhow::anyhow!("Cannot open the native desktop window: {e}"))
}
impl Desktop {
    fn new(state: app::State, runtime: tokio::runtime::Handle, ctx: &egui::Context) -> Self {
        ctx.set_theme(egui::ThemePreference::System);
        let recovery = runtime.block_on(desktop_api::recover(&state));
        let (sender, receiver) = mpsc::channel();
        let mut result = Self {
            state,
            runtime,
            sender,
            receiver,
            data: Snapshot::default(),
            tools: Tools::default(),
            view: View::New,
            search: String::new(),
            title: String::new(),
            system_audio: true,
            mic: true,
            permission: false,
            provider: String::new(),
            ai_consent: false,
            notes_edit: String::new(),
            notes_dirty: false,
            output: String::new(),
            notice: String::new(),
            theme: "system".into(),
            hide_completed: false,
            busy: false,
            epoch: 0,
            queued: VecDeque::new(),
            notes_save_pending: false,
            #[cfg(not(windows))]
            import_path: String::new(),
            refreshing: false,
            last_refresh: Instant::now() - Duration::from_secs(5),
            pending_navigation: None,
            delete_confirm: None,
            close_confirm: false,
            closing: false,
            #[cfg(windows)]
            capture: None,
            #[cfg(windows)]
            recording_title: String::new(),
            #[cfg(windows)]
            recording_warning: String::new(),
            #[cfg(windows)]
            player: None,
        };
        result.notice = match recovery {
            Ok((0, 0)) => String::new(),
            Ok((count, retained)) => format!(
                "Recovered {count} interrupted recording(s); {retained} unreadable capture file(s) retained in the data folder."
            ),
            Err(error) => format!("Cannot check recording recovery: {error}"),
        };
        result.check_tools(ctx);
        result.refresh(ctx);
        result
    }
    fn check_tools(&self, ctx: &egui::Context) {
        let state = self.state.clone();
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        self.runtime.spawn(async move {
            let (speech, codex, claude) = tokio::join!(
                state.transcriber.available(),
                state.agents.detect("codex"),
                state.agents.detect("claude")
            );
            let _ = sender.send(Message::Tools(Tools {
                speech,
                codex,
                claude,
                checked: true,
            }));
            ctx.request_repaint();
        });
    }
    fn refresh(&mut self, ctx: &egui::Context) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.last_refresh = Instant::now();
        let state = self.state.clone();
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        let view = self.view;
        let epoch = self.epoch;
        self.runtime.spawn(async move {
            let data = desktop_api::snapshot(&state).await;
            let output = if let View::Session(id) = view {
                Some((id, desktop_api::output(&state, id).await))
            } else {
                None
            };
            let _ = sender.send(Message::Snapshot(epoch, data, output));
            ctx.request_repaint();
        });
    }
    fn dispatch(&mut self, command: Command, ctx: &egui::Context) {
        if self.busy {
            self.queued.push_back(command);
            return;
        }
        self.epoch += 1;
        self.busy = true;
        let state = self.state.clone();
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        self.runtime.spawn(async move {
            let result = desktop_api::execute(&state, command).await;
            let _ = sender.send(Message::Finished(result));
            ctx.request_repaint();
        });
    }
    fn navigate(&mut self, view: View) {
        if self.notes_dirty {
            self.pending_navigation = Some(view);
            return;
        }
        self.stop_playback();
        self.view = view;
        self.ai_consent = false;
        self.output.clear();
        if let View::Session(id) = view {
            self.notes_edit = self
                .data
                .sessions
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.notes.clone())
                .unwrap_or_default();
        }
        self.last_refresh = Instant::now() - Duration::from_secs(5);
    }
    fn theme(ctx: &egui::Context, value: &str) {
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(match value {
            "dark" => egui::SystemTheme::Dark,
            "light" => egui::SystemTheme::Light,
            _ => egui::SystemTheme::SystemDefault,
        }));
        ctx.set_theme(match value {
            "light" => egui::ThemePreference::Light,
            "dark" => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        });
    }
    fn recording(&self) -> bool {
        #[cfg(windows)]
        {
            self.capture.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    fn stop_capture(&mut self) {
        #[cfg(windows)]
        if let Some(capture) = &self.capture {
            capture
                .stop
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    fn begin_capture(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        {
            if !self.permission || !(self.system_audio || self.mic) || self.capture.is_some() {
                return;
            }
            self.stop_playback();
            self.recording_title = self.title.clone();
            self.recording_warning.clear();
            let path = self
                .state
                .root
                .join(format!(".capture-{}.wav", Uuid::new_v4()));
            let ctx = ctx.clone();
            self.capture = Some(crate::capture::Capture::start(
                path,
                self.system_audio,
                self.mic,
                self.state.overlay.clone(),
                move || ctx.request_repaint(),
            ));
            self.notice = "Starting native Windows audio capture…".into();
        }
        #[cfg(not(windows))]
        {
            let _ = ctx;
            self.notice = "Native recording is available in the Windows build.".into();
        }
    }
    fn stop_playback(&mut self) {
        #[cfg(windows)]
        {
            self.player.take();
        }
    }
    fn import(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "Audio or video",
                &["wav", "mp3", "m4a", "mp4", "webm", "ogg", "opus", "flac"],
            )
            .pick_file()
        {
            let title = if self.title.trim().is_empty() {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            } else {
                self.title.clone()
            };
            self.dispatch(
                Command::Import {
                    path,
                    title,
                    owned_capture: false,
                },
                ctx,
            );
        }
        #[cfg(not(windows))]
        {
            if self.import_path.trim().is_empty() {
                self.notice = "Enter a file path before importing.".into();
                return;
            }
            self.dispatch(
                Command::Import {
                    path: std::path::PathBuf::from(self.import_path.trim()),
                    title: self.title.clone(),
                    owned_capture: false,
                },
                ctx,
            );
        }
    }
    fn export(&mut self, id: Uuid, notes: bool, ctx: &egui::Context) {
        #[cfg(windows)]
        {
            let name = if notes {
                "meeting-notes.md"
            } else {
                "transcript.md"
            };
            if let Some(destination) = rfd::FileDialog::new().set_file_name(name).save_file() {
                let source = self.state.root.join(id.to_string()).join(if notes {
                    "notes.md"
                } else {
                    "transcript.md"
                });
                let sender = self.sender.clone();
                let ctx = ctx.clone();
                self.runtime.spawn(async move {
                    let message = match tokio::fs::copy(source, destination).await {
                        Ok(_) => "Exported".into(),
                        Err(e) => format!("Cannot export: {e}"),
                    };
                    let _ = sender.send(Message::Notice(message));
                    ctx.request_repaint();
                });
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (id, notes, ctx);
            self.notice = "Markdown is saved in the session directory.".into();
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.receiver.try_recv() {
            match message {
                Message::Tools(tools) => {
                    if self.provider.is_empty() {
                        self.provider = if tools.codex.installed {
                            "codex"
                        } else if tools.claude.installed {
                            "claude"
                        } else {
                            ""
                        }
                        .into();
                    }
                    self.tools = tools;
                }
                Message::Notice(value) => self.notice = value,
                Message::Snapshot(epoch, result, output) => {
                    if epoch != self.epoch {
                        self.refreshing = false;
                        self.last_refresh = Instant::now() - Duration::from_secs(5);
                        continue;
                    }
                    self.refreshing = false;
                    match result {
                        Ok(data) => {
                            if !self.busy && self.theme != data.theme {
                                self.theme = data.theme.clone();
                                Self::theme(ctx, &self.theme);
                            }
                            if let View::Session(id) = self.view {
                                if let Some(r) = data.sessions.iter().find(|r| r.id == id) {
                                    if !self.notes_dirty {
                                        self.notes_edit = r.notes.clone();
                                    }
                                } else {
                                    self.view = View::New;
                                }
                            }
                            self.data = data;
                        }
                        Err(error) => self.notice = error,
                    }
                    if let Some((id, text)) = output
                        && self.view == View::Session(id)
                    {
                        self.output = text;
                    }
                }
                Message::Finished(result) => {
                    self.busy = false;
                    match result {
                        Ok(outcome) => {
                            if self.notes_save_pending {
                                self.notes_dirty = false;
                            }
                            self.notice = outcome.message;
                            if let Some(file) = outcome.file {
                                #[cfg(windows)]
                                if outcome
                                    .session
                                    .is_some_and(|id| self.view == View::Session(id))
                                {
                                    self.player = Some(crate::playback::Player::start(file));
                                }
                                #[cfg(not(windows))]
                                {
                                    let _ = file;
                                }
                            } else if let Some(id) = outcome.session {
                                self.navigate(View::Session(id));
                            }
                        }
                        Err(error) => {
                            self.notice = error;
                            self.closing = false;
                        }
                    }
                    self.notes_save_pending = false;
                    if let Some(next) = self.queued.pop_front() {
                        self.dispatch(next, ctx);
                    }
                    self.refresh(ctx);
                }
            }
        }
        #[cfg(windows)]
        {
            if let Some(result) = self.capture.as_ref().and_then(|c| c.poll()) {
                self.capture.take();
                match result {
                    Ok(record) => {
                        self.recording_warning = record.warning.unwrap_or_default();
                        self.dispatch(
                            Command::Import {
                                path: record.path,
                                title: self.recording_title.clone(),
                                owned_capture: true,
                            },
                            ctx,
                        );
                    }
                    Err(error) => {
                        self.notice = format!(
                            "Recording failed: {error}. Any recovery audio remains in the data folder."
                        );
                        self.closing = false;
                    }
                }
            }
            if let Some(player) = &self.player {
                if let Some(error) = player.error() {
                    self.notice = format!("Playback failed: {error}");
                }
                if player.finished.load(std::sync::atomic::Ordering::SeqCst) {
                    self.player.take();
                }
            }
        }
        if self.last_refresh.elapsed() > Duration::from_secs(2) && !self.refreshing {
            self.refresh(ctx);
        }
        if self.closing && !self.recording() && !self.busy && !self.notes_dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn render(&mut self, ctx: &egui::Context) {
        self.poll(ctx);
        if ctx.input(|i| i.viewport().close_requested())
            && !self.closing
            && (self.recording() || self.notes_dirty || self.busy)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_confirm = true;
        }
        egui::SidePanel::left("library")
            .resizable(true)
            .default_width(245.0)
            .min_width(190.0)
            .show(ctx, |ui| {
                ui.add_space(18.0);
                ui.heading("OpenAIRecorder");
                ui.label(egui::RichText::new("YOUR CONVERSATIONS").small().weak());
                ui.add_space(20.0);
                if ui
                    .add_sized(
                        [ui.available_width(), 34.0],
                        egui::Button::new("+ New conversation"),
                    )
                    .clicked()
                {
                    self.navigate(View::New);
                }
                if ui
                    .selectable_label(
                        self.view == View::Actions,
                        format!(
                            "Global actions ({})",
                            self.data.actions.iter().filter(|g| !g.action.done).count()
                        ),
                    )
                    .clicked()
                {
                    self.navigate(View::Actions);
                }
                if ui
                    .selectable_label(self.view == View::Settings, "Settings")
                    .clicked()
                {
                    self.navigate(View::Settings);
                }
                ui.separator();
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search sessions & transcripts"),
                );
                ui.add_space(12.0);
                let query = self.search.to_lowercase();
                let mut selected = None;
                egui::ScrollArea::vertical()
                    .id_salt("library-scroll")
                    .show(ui, |ui| {
                        for r in &self.data.sessions {
                            if !format!(
                                "{} {} {} {}",
                                r.title,
                                r.transcript,
                                r.notes,
                                r.actions
                                    .iter()
                                    .map(|a| a.title.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            )
                            .to_lowercase()
                            .contains(&query)
                            {
                                continue;
                            }
                            let label =
                                format!("{}{}", if r.pinned { "[Pinned] " } else { "" }, r.title);
                            if ui
                                .selectable_label(self.view == View::Session(r.id), label)
                                .on_hover_text(&r.created_at)
                                .clicked()
                            {
                                selected = Some(r.id);
                            }
                            ui.label(egui::RichText::new(&r.status).small().weak());
                            ui.add_space(8.0);
                        }
                        if self.data.sessions.is_empty() {
                            ui.weak("Your first conversation starts here.");
                        }
                    });
                if let Some(id) = selected {
                    self.navigate(View::Session(id));
                }
            });
        egui::TopBottomPanel::top("status").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.weak("LOCAL FIRST  /  NATIVE DESKTOP");
                if self.busy {
                    ui.spinner();
                }
                if !self.notice.is_empty() {
                    ui.label(&self.notice);
                }
            });
            if self.recording() {
                ui.horizontal(|ui| {
                    #[cfg(windows)]
                    if let Some(c) = &self.capture {
                        let s = c.clock.elapsed().as_secs();
                        if !c.active.load(std::sync::atomic::Ordering::SeqCst) {
                            ui.weak("Starting audio devices…");
                        }
                        ui.colored_label(
                            egui::Color32::from_rgb(205, 75, 65),
                            format!(
                                "{}  {:02}:{:02}",
                                if c.paused.load(std::sync::atomic::Ordering::SeqCst) {
                                    "PAUSED"
                                } else {
                                    "RECORDING"
                                },
                                s / 60,
                                s % 60
                            ),
                        );
                        ui.add(
                            egui::ProgressBar::new(f32::from_bits(
                                c.level.load(std::sync::atomic::Ordering::Relaxed),
                            ))
                            .desired_width(100.0),
                        );
                        let paused = c.paused.load(std::sync::atomic::Ordering::SeqCst);
                        if ui
                            .add_enabled(
                                c.active.load(std::sync::atomic::Ordering::SeqCst),
                                egui::Button::new(if paused { "Resume" } else { "Pause" }),
                            )
                            .clicked()
                        {
                            c.paused.store(!paused, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    if ui.button("Stop & save").clicked() {
                        self.stop_capture();
                    }
                });
            }
            ui.add_space(8.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(16.0);
            egui::ScrollArea::vertical()
                .id_salt("content-scroll")
                .show(ui, |ui| match self.view {
                    View::New => self.new_view(ui, ctx),
                    View::Session(id) => self.session_view(ui, ctx, id),
                    View::Actions => self.actions_view(ui, ctx),
                    View::Settings => self.settings_view(ui, ctx),
                });
        });
        self.dialogs(ctx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    fn new_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading(egui::RichText::new("Be present. Keep the important parts.").size(30.0));
        ui.label(
            "Capture the conversation on your computer, then let your signed-in agent take notes.",
        );
        ui.add_space(24.0);
        egui::Frame::group(ui.style()).inner_margin(20.0).show(ui,|ui|{
            ui.heading("New conversation");ui.add_space(12.0);ui.label("Conversation name");ui.add(egui::TextEdit::singleline(&mut self.title).hint_text("Monday product sync").desired_width(f32::INFINITY));
            ui.add_space(12.0);ui.checkbox(&mut self.system_audio,"Capture meeting/system audio");ui.checkbox(&mut self.mic,"Include microphone");
            ui.weak("Meeting audio uses your Windows default playback device (speakers/headphones). Make sure Teams, Slack, or Zoom uses that device. Use headphones to reduce echo.");
            ui.add_space(12.0);ui.checkbox(&mut self.permission,"I have informed the participants and have permission to record.");
            ui.add_space(12.0);
            if ui.add_enabled(cfg!(windows)&&self.permission&&(self.system_audio||self.mic)&&!self.recording()&&!self.busy,egui::Button::new("Start recording").min_size(egui::vec2(160.0,35.0))).clicked(){self.begin_capture(ctx);}
            ui.weak("A movable, always-on-top recording widget appears when capture starts. You can minimize this window; recording and its Stop button continue working.");
            #[cfg(not(windows))] ui.weak("Linux desktop preview: native audio capture is Windows-only.");
        });
        ui.add_space(20.0);
        ui.heading("Already have a recording?");
        ui.label("Import an audio or video file up to 150 MB.");
        #[cfg(not(windows))]
        ui.add(
            egui::TextEdit::singleline(&mut self.import_path)
                .hint_text("Full file path (Linux preview)"),
        );
        if ui
            .add_enabled(!self.busy, egui::Button::new("Import recording…"))
            .clicked()
        {
            self.import(ctx);
        }
    }
    fn session_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: Uuid) {
        let Some(r) = self.data.sessions.iter().find(|r| r.id == id).cloned() else {
            ui.spinner();
            return;
        };
        let busy = self.busy || r.status == "processing";
        ui.horizontal_wrapped(|ui| {
            ui.heading(&r.title);
            ui.weak(&r.status);
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(if r.pinned {
                        "Unpin session"
                    } else {
                        "Pin session"
                    }),
                )
                .clicked()
            {
                self.dispatch(
                    Command::Pin {
                        id,
                        pinned: !r.pinned,
                    },
                    ctx,
                );
            }
        });
        ui.weak(&r.created_at);
        if let Some(error) = &r.error {
            ui.colored_label(egui::Color32::from_rgb(205, 90, 75), error);
        }
        #[cfg(windows)]
        if !self.recording_warning.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(205, 140, 70),
                &self.recording_warning,
            );
        }
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !busy && !self.recording() && cfg!(windows),
                    egui::Button::new("Play recording"),
                )
                .clicked()
            {
                self.stop_playback();
                self.dispatch(Command::Playback(id), ctx);
            }
            if ui.button("Stop playback").clicked() {
                self.stop_playback();
            }
            if ui
                .add_enabled(
                    !busy && self.tools.speech,
                    egui::Button::new("Generate local transcript"),
                )
                .clicked()
            {
                self.dispatch(Command::Transcribe(id), ctx);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Delete session…"))
                .clicked()
            {
                self.delete_confirm = Some(id);
            }
        });
        if !self.tools.speech {
            ui.weak(
                "Local speech tools are not ready. See Settings for installation instructions.",
            );
        }
        ui.separator();
        ui.heading("Notes & decisions");
        if r.notes.is_empty() {
            ui.weak("Run your agent to generate notes.");
        } else {
            markdown(ui, &r.notes);
        }
        egui::CollapsingHeader::new("Edit notes").show(ui, |ui| {
            self.notes_dirty |= ui
                .add(
                    egui::TextEdit::multiline(&mut self.notes_edit)
                        .desired_width(f32::INFINITY)
                        .desired_rows(12),
                )
                .changed();
            if ui
                .add_enabled(!busy && self.notes_dirty, egui::Button::new("Save notes"))
                .clicked()
            {
                self.dispatch(
                    Command::Notes {
                        id,
                        notes: self.notes_edit.clone(),
                    },
                    ctx,
                );
                self.notes_save_pending = true;
            }
        });
        if ui.button("Export notes as Markdown…").clicked() {
            self.export(id, true, ctx);
        }
        ui.separator();
        ui.heading("Session action items");
        for action in &r.actions {
            ui.horizontal_wrapped(|ui| {
                let mut done = action.done;
                if ui
                    .add_enabled(!busy, egui::Checkbox::new(&mut done, &action.title))
                    .changed()
                {
                    self.dispatch(
                        Command::Complete {
                            id,
                            action: action.id,
                            done,
                        },
                        ctx,
                    );
                }
                let copied = self
                    .data
                    .actions
                    .iter()
                    .any(|g| g.source_session == id && g.source_action == action.id);
                if ui
                    .add_enabled(
                        !self.busy && !copied,
                        egui::Button::new(if copied {
                            "Copied"
                        } else {
                            "Copy to global actions"
                        }),
                    )
                    .clicked()
                {
                    self.dispatch(
                        Command::Copy {
                            id,
                            action: action.id,
                        },
                        ctx,
                    );
                }
            });
            ui.weak(format!(
                "Owner: {}  |  Due: {}",
                action.owner.as_deref().unwrap_or("unspecified"),
                action.due.as_deref().unwrap_or("unspecified")
            ));
        }
        if r.actions.is_empty() {
            ui.weak("Your agent will extract action items here.");
        }
        ui.separator();
        ui.heading("Your subscription agent");
        self.agent_options(ui);
        ui.checkbox(
            &mut self.ai_consent,
            "Send this transcript to my selected agent using my signed-in account.",
        );
        let installed = match self.provider.as_str() {
            "codex" => self.tools.codex.installed,
            "claude" => self.tools.claude.installed,
            _ => false,
        };
        if ui
            .add_enabled(
                !busy
                    && installed
                    && !r.transcript.is_empty()
                    && self.ai_consent
                    && !self.notes_dirty,
                egui::Button::new("Generate notes & actions"),
            )
            .clicked()
        {
            self.dispatch(
                Command::Generate {
                    id,
                    provider: self.provider.clone(),
                    consent: true,
                },
                ctx,
            );
        }
        ui.weak("Uses your CLI subscription sign-in and its usage limits. Raw audio stays on your computer. Installation and sign-in instructions are in Settings.");
        egui::CollapsingHeader::new("Agent output & diagnostics").show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(250.0)
                .id_salt("agent-log")
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.output)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
        });
        ui.separator();
        egui::CollapsingHeader::new("Full transcript").show(ui, |ui| {
            if r.transcript.is_empty() {
                ui.weak("Generate a local transcript first.");
            } else {
                ui.label(&r.transcript);
            }
            if ui.button("Copy transcript").clicked() {
                ctx.copy_text(r.transcript.clone());
            }
            if ui.button("Export transcript…").clicked() {
                self.export(id, false, ctx);
            }
        });
    }
    fn agent_options(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled(
                self.tools.codex.installed,
                egui::RadioButton::new(self.provider == "codex", "Codex / ChatGPT"),
            )
            .clicked()
            .then(|| self.provider = "codex".into());
            ui.add_enabled(
                self.tools.claude.installed,
                egui::RadioButton::new(self.provider == "claude", "Claude Code"),
            )
            .clicked()
            .then(|| self.provider = "claude".into());
        });
    }
    fn actions_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Your global actions");
        ui.label("Copied actions retain their source and survive session deletion. Completion is independent of the session checklist.");
        ui.checkbox(&mut self.hide_completed, "Hide completed");
        ui.separator();
        let actions = self.data.actions.clone();
        for global in actions {
            if self.hide_completed && global.action.done {
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                let mut done = global.action.done;
                if ui
                    .add_enabled(
                        !self.busy,
                        egui::Checkbox::new(&mut done, &global.action.title),
                    )
                    .changed()
                {
                    self.dispatch(
                        Command::GlobalComplete {
                            id: global.id,
                            done,
                        },
                        ctx,
                    );
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Remove"))
                    .clicked()
                {
                    self.dispatch(Command::GlobalRemove(global.id), ctx);
                }
            });
            ui.weak(format!(
                "Owner: {}  |  Due: {}",
                global.action.owner.as_deref().unwrap_or("unspecified"),
                global.action.due.as_deref().unwrap_or("unspecified")
            ));
            if self
                .data
                .sessions
                .iter()
                .any(|r| r.id == global.source_session)
            {
                if ui.link(format!("From: {}", global.source_title)).clicked() {
                    self.navigate(View::Session(global.source_session));
                }
            } else {
                ui.weak(format!("From: {} (session deleted)", global.source_title));
            }
            ui.separator();
        }
        if self.data.actions.is_empty() {
            ui.weak("Copy actions from a session to manage them here.");
        }
    }
    fn settings_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Settings");
        ui.add_space(12.0);
        ui.heading("Appearance");
        ui.label("System follows your computer's appearance, including changes while the app is running.");
        ui.horizontal(|ui| {
            for (key, name) in [("light", "Light"), ("dark", "Dark"), ("system", "System")] {
                if ui
                    .add_enabled(!self.busy, egui::RadioButton::new(self.theme == key, name))
                    .clicked()
                {
                    self.theme = key.into();
                    Self::theme(ctx, key);
                    self.dispatch(Command::Theme(key.into()), ctx);
                }
            }
        });
        ui.separator();
        ui.heading("Local transcription");
        ui.label(if !self.tools.checked {
            "Checking tools…"
        } else if self.tools.speech {
            "Whisper, its speech model, and ffmpeg are ready."
        } else {
            "Local speech tools need setup."
        });
        ui.weak("The Windows ZIP includes Whisper and the speech model. Install ffmpeg and the Visual C++ runtime if they are missing:");
        for command in [
            "winget install --id Gyan.FFmpeg --exact",
            "winget install --id Microsoft.VCRedist.2015+.x64 --exact",
        ] {
            copy_command(ui, ctx, command);
        }
        ui.separator();
        ui.heading("Subscription agents");
        for (name, info, install, login) in [
            (
                "Codex",
                &self.tools.codex,
                "npm install -g @openai/codex",
                "codex login",
            ),
            (
                "Claude Code",
                &self.tools.claude,
                "winget install --id Anthropic.ClaudeCode --exact",
                "claude auth login",
            ),
        ] {
            ui.strong(format!(
                "{name}: {}",
                if info.installed {
                    "Detected"
                } else {
                    "Not detected"
                }
            ));
            if let Some(version) = &info.version {
                ui.weak(version);
            }
            if !info.installed {
                copy_command(ui, ctx, install);
            }
            copy_command(ui, ctx, login);
        }
        ui.weak("Restart the app after installing tools so it receives the updated PATH. Detection confirms installation; subscription sign-in and quota are checked when notes are generated.");
        if ui.button("Recheck installed tools").clicked() {
            self.check_tools(ctx);
        }
        ui.separator();
        ui.heading("Data on this computer");
        ui.label(self.state.root.display().to_string());
        ui.weak("Audio, transcripts, Markdown notes, settings, and actions are stored here. Existing recordings from the earlier portable build are reused when found next to the executable.");
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.pending_navigation.is_some() {
            egui::Window::new("Unsaved notes")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Save your notes before switching views, or discard the edits.");
                    if ui.button("Discard edits and continue").clicked() {
                        self.notes_dirty = false;
                        if let Some(view) = self.pending_navigation.take() {
                            self.navigate(view);
                        }
                    }
                    if ui.button("Keep editing").clicked() {
                        self.pending_navigation = None;
                    }
                });
        }
        if let Some(id) = self.delete_confirm {
            egui::Window::new("Delete this session?").collapsible(false).resizable(false).show(ctx,|ui|{
            ui.label("Its audio, transcript and notes will be deleted. Copied global actions remain.");
            if ui.add_enabled(!self.busy,egui::Button::new("Delete session")).clicked(){self.delete_confirm=None;self.notes_dirty=false;self.dispatch(Command::Delete(id),ctx);}
            if ui.button("Cancel").clicked(){self.delete_confirm=None;}
        });
        }
        if self.close_confirm {
            egui::Window::new("Close OpenAIRecorder?").collapsible(false).resizable(false).show(ctx,|ui|{
            ui.label("An active recording will stop and be saved before closing. Save or discard any note edits first.");
            if ui.add_enabled(!self.notes_dirty&&!self.busy,egui::Button::new("Stop, save, and close")).clicked(){self.stop_capture();self.closing=true;self.close_confirm=false;}
            if ui.button("Keep app open").clicked(){self.close_confirm=false;}
        });
        }
    }
}
impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render(ctx);
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.stop_playback();
        #[cfg(windows)]
        if let Some(mut capture) = self.capture.take()
            && let Some(Ok(result)) = capture.finish()
        {
            let _ = self.runtime.block_on(desktop_api::execute(
                &self.state,
                Command::Import {
                    path: result.path,
                    title: self.recording_title.clone(),
                    owned_capture: true,
                },
            ));
        }
        self.state.overlay.hide();
    }
}
fn copy_command(ui: &mut egui::Ui, ctx: &egui::Context, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.code(text);
        if ui.small_button("Copy").clicked() {
            ctx.copy_text(text.into());
        }
    });
}
fn markdown(ui: &mut egui::Ui, text: &str) {
    for line in text.lines() {
        if line.trim().is_empty() {
            ui.add_space(6.0);
        } else if let Some(heading) = line.strip_prefix("# ").or_else(|| line.strip_prefix("## ")) {
            ui.heading(heading);
        } else if let Some(heading) = line.strip_prefix("### ") {
            ui.strong(heading);
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            ui.label(format!("• {item}"));
        } else {
            ui.label(line);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_panels_render_headlessly_and_themes_switch() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let root = std::env::temp_dir().join(Uuid::new_v4().to_string());
        std::fs::create_dir(&root).unwrap();
        let state = app::State {
            root: root.clone(),
            transcriber: crate::transcription::Transcriber::from_env(),
            agents: crate::agent::Agents {
                codex: root.join("missing-codex"),
                claude: root.join("missing-claude"),
            },
            overlay: crate::overlay::Overlay::new(),
            jobs: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        };
        let ctx = egui::Context::default();
        let mut app = Desktop::new(state, runtime.handle().clone(), &ctx);
        for view in [View::New, View::Actions, View::Settings] {
            app.view = view;
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 850.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.render(ctx),
            );
            assert!(!out.shapes.is_empty());
        }
        Desktop::theme(&ctx, "dark");
        assert!(ctx.style().visuals.dark_mode);
        Desktop::theme(&ctx, "light");
        assert!(!ctx.style().visuals.dark_mode);
        drop(app);
        runtime.shutdown_timeout(Duration::from_secs(1));
        std::fs::remove_dir_all(root).unwrap();
    }
}
