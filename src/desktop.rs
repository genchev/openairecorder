use crate::design::{self, ACCENT};
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
    session_tab: usize,
    markdown_cache: egui_commonmark::CommonMarkCache,
    #[cfg(not(windows))]
    markdown_path: String,
    speech_language: String,
    speech_model: String,
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
        design::install_fonts(ctx);
        ctx.set_theme(egui::ThemePreference::System);
        let recovery = runtime.block_on(desktop_api::recover(&state));
        let (sender, receiver) = mpsc::channel();
        let speech = crate::transcription::SpeechSettings::load(&state.root);
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
            session_tab: 0,
            markdown_cache: egui_commonmark::CommonMarkCache::default(),
            #[cfg(not(windows))]
            markdown_path: String::new(),
            speech_language: speech.language,
            speech_model: speech.model,
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
                state.transcriber.available(&state.root),
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
        self.session_tab = 0;
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
                            if self.notice == "Speech preferences saved" {
                                self.check_tools(ctx);
                            }
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
        design::style(ctx);
        if ctx.input(|i| i.viewport().close_requested())
            && !self.closing
            && (self.recording() || self.notes_dirty || self.busy)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_confirm = true;
        }
        let bg = if ctx.style().visuals.dark_mode {
            egui::Color32::from_rgb(25, 27, 38)
        } else {
            egui::Color32::from_rgb(241, 242, 249)
        };
        egui::SidePanel::left("library")
            .resizable(false)
            .exact_width(244.0)
            .frame(egui::Frame::new().fill(bg).inner_margin(18.0))
            .show(ctx, |ui| {
                ui.add_space(10.0);
                design::logo(ui);
                ui.add_space(32.0);
                if ui
                    .add_sized(
                        [ui.available_width(), 42.0],
                        design::primary("+  New recording"),
                    )
                    .clicked()
                {
                    self.navigate(View::New);
                }
                ui.add_space(12.0);
                if nav(ui, "Workspace", self.view == View::New).clicked() {
                    self.navigate(View::New);
                }
                let open = self.data.actions.iter().filter(|g| !g.action.done).count();
                if nav(
                    ui,
                    &format!("Global actions    {open}"),
                    self.view == View::Actions,
                )
                .clicked()
                {
                    self.navigate(View::Actions);
                }
                if nav(ui, "Settings", self.view == View::Settings).clicked() {
                    self.navigate(View::Settings);
                }
                ui.add_space(26.0);
                design::eyebrow(ui, "YOUR LIBRARY");
                ui.add_space(4.0);
                ui.add_sized(
                    [ui.available_width(), 36.0],
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search conversations…")
                        .margin(egui::vec2(10.0, 9.0)),
                );
                ui.add_space(10.0);
                let query = self.search.to_lowercase();
                let mut selected = None;
                let mut matches = 0;
                egui::ScrollArea::vertical()
                    .id_salt("library-scroll")
                    .max_height(ui.available_height() - 78.0)
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
                            matches += 1;
                            let chosen = self.view == View::Session(r.id);
                            let fill = if chosen {
                                if ui.visuals().dark_mode {
                                    egui::Color32::from_rgb(47, 39, 75)
                                } else {
                                    egui::Color32::from_rgb(230, 224, 251)
                                }
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            let label = format!(
                                "{}{}\n{}",
                                if r.pinned { "•  " } else { "" },
                                short(&r.title, 25),
                                date(&r.created_at)
                            );
                            if ui
                                .add_sized(
                                    [ui.available_width(), 58.0],
                                    egui::Button::new(egui::RichText::new(label).size(13.0))
                                        .fill(fill)
                                        .stroke(egui::Stroke::NONE)
                                        .corner_radius(9),
                                )
                                .on_hover_text(&r.title)
                                .clicked()
                            {
                                selected = Some(r.id);
                            }
                        }
                        if matches == 0 {
                            design::subtitle(
                                ui,
                                if query.is_empty() {
                                    "Your conversations will appear here."
                                } else {
                                    "No matching conversations."
                                },
                            );
                        }
                    });
                if let Some(id) = selected {
                    self.navigate(View::Session(id));
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(8.0);
                    design::subtitle(ui, "Your recordings. Your space.");
                    design::pill(ui, "SAVED ON THIS COMPUTER", ACCENT);
                });
            });
        egui::TopBottomPanel::top("workspace-header")
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(34, 18)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    design::eyebrow(ui, "MY WORKSPACE");
                    ui.label(egui::RichText::new("/").color(design::muted(ui)));
                    ui.label(match self.view {
                        View::New => "Overview",
                        View::Session(_) => "Conversation",
                        View::Actions => "Global actions",
                        View::Settings => "Settings",
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.busy {
                            ui.spinner();
                            ui.label("Saving…");
                        } else {
                            design::pill(ui, "PRIVATE WORKSPACE", ACCENT);
                        }
                    });
                });
            });
        if !self.notice.is_empty() {
            egui::TopBottomPanel::bottom("notice")
                .frame(
                    egui::Frame::new()
                        .fill(ctx.style().visuals.window_fill)
                        .inner_margin(egui::Margin::symmetric(24, 10)),
                )
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(&self.notice);
                        if ui.small_button("Dismiss").clicked() {
                            self.notice.clear();
                        }
                    });
                });
        }
        if self.recording() {
            egui::TopBottomPanel::bottom("recording-controls")
                .frame(
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(36, 30, 62))
                        .inner_margin(egui::Margin::symmetric(28, 16)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        #[cfg(windows)]
                        if let Some(c) = &self.capture {
                            let paused = c.paused.load(std::sync::atomic::Ordering::SeqCst);
                            let s = c.clock.elapsed().as_secs();
                            ui.label(
                                egui::RichText::new(if paused {
                                    "PAUSED"
                                } else {
                                    "● RECORDING"
                                })
                                .color(design::MINT)
                                .strong(),
                            );
                            ui.label(
                                egui::RichText::new(format!("{:02}:{:02}", s / 60, s % 60))
                                    .color(egui::Color32::WHITE)
                                    .size(24.0),
                            );
                            design::waveform(
                                ui,
                                egui::vec2(130.0, 36.0),
                                Some(if paused {
                                    0.0
                                } else {
                                    f32::from_bits(
                                        c.level.load(std::sync::atomic::Ordering::Relaxed),
                                    )
                                }),
                            );
                            if ui
                                .add_enabled(
                                    c.active.load(std::sync::atomic::Ordering::SeqCst),
                                    design::primary(if paused { "Resume" } else { "Pause" }),
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
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(34, 26)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("content-scroll")
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        match self.view {
                            View::New => self.new_view(ui, ctx),
                            View::Session(id) => self.session_view(ui, ctx, id),
                            View::Actions => self.actions_view(ui, ctx),
                            View::Settings => self.settings_view(ui, ctx),
                        }
                        ui.add_space(24.0);
                    });
            });
        self.dialogs(ctx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    fn new_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            egui::RichText::new("Make room for the conversation.")
                .size(34.0)
                .strong(),
        );
        design::subtitle(
            ui,
            "Capture the moment. Keep the notes, decisions, and next steps together.",
        );
        ui.add_space(24.0);
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(39, 31, 66))
            .corner_radius(18)
            .inner_margin(26.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("A LITTLE MORE PRESENCE")
                                .size(11.0)
                                .color(design::MINT)
                                .strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Listen now. Revisit later.")
                                .size(25.0)
                                .color(egui::Color32::WHITE)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new("One place for every conversation.")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(190, 183, 213)),
                        );
                    });
                    if ui.available_width() > 140.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            design::waveform(
                                ui,
                                egui::vec2(ui.available_width().min(210.0), 68.0),
                                None,
                            )
                        });
                    }
                });
            });
        ui.add_space(20.0);
        design::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.heading("Start a recording");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    design::pill(ui, "AUDIO ONLY", ACCENT)
                });
            });
            ui.add_space(8.0);
            design::eyebrow(ui, "CONVERSATION NAME");
            ui.add_sized(
                [ui.available_width(), 42.0],
                egui::TextEdit::singleline(&mut self.title)
                    .hint_text("e.g. Weekly team sync")
                    .margin(egui::vec2(12.0, 11.0)),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.system_audio, "Meeting audio");
                ui.add_space(14.0);
                ui.checkbox(&mut self.mic, "Microphone");
            });
            design::subtitle(
                ui,
                "Teams, Zoom, Slack, and other audio playing through your default Windows output.",
            );
            ui.checkbox(
                &mut self.permission,
                "Everyone in this conversation knows I'm recording.",
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        cfg!(windows)
                            && self.permission
                            && (self.system_audio || self.mic)
                            && !self.recording()
                            && !self.busy,
                        design::primary("●  Start recording").min_size(egui::vec2(175.0, 42.0)),
                    )
                    .clicked()
                {
                    self.begin_capture(ctx);
                }
                design::subtitle(ui, "Pause anytime. Floating controls stay within reach.");
            });
            #[cfg(not(windows))]
            design::subtitle(
                ui,
                "Desktop preview · Recording is available in the Windows build.",
            );
        });
        ui.add_space(18.0);
        ui.horizontal_wrapped(|ui| {
            design::subtitle(ui, "Already recorded it?");
            if ui
                .add_enabled(!self.busy, egui::Button::new("Import audio…"))
                .clicked()
            {
                self.import(ctx);
            }
            #[cfg(not(windows))]
            ui.add(
                egui::TextEdit::singleline(&mut self.import_path)
                    .hint_text("Audio path for preview")
                    .desired_width(240.0),
            );
        });
        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.heading("Recent conversations");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                design::subtitle(ui, &format!("{} sessions", self.data.sessions.len()))
            });
        });
        ui.add_space(8.0);
        let sessions = self.data.sessions.clone();
        if sessions.is_empty() {
            design::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add_space(12.0);
                ui.strong("Your next conversation belongs here.");
                design::subtitle(
                    ui,
                    "Start a recording or import audio to create your first session.",
                );
                ui.add_space(12.0);
            });
        } else {
            let columns = if ui.available_width() > 700.0 { 2 } else { 1 };
            let width = (ui.available_width() - 14.0 * (columns - 1) as f32) / columns as f32;
            egui::Grid::new("recent-grid")
                .num_columns(columns)
                .spacing(egui::vec2(14.0, 14.0))
                .show(ui, |ui| {
                    for (i, r) in sessions.iter().take(8).enumerate() {
                        design::card(ui).show(ui, |ui| {
                            ui.set_width((width - 44.0).max(120.0));
                            design::eyebrow(
                                ui,
                                &format!(
                                    "{}{}",
                                    if r.pinned { "PINNED  ·  " } else { "" },
                                    date(&r.created_at)
                                ),
                            );
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(&r.title).size(18.0).strong(),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.navigate(View::Session(r.id));
                            }
                            let open = r.actions.iter().filter(|a| !a.done).count();
                            design::subtitle(
                                ui,
                                &format!(
                                    "{}  ·  {open} open actions",
                                    if r.notes.is_empty() {
                                        "Recording saved"
                                    } else {
                                        "Notes ready"
                                    }
                                ),
                            );
                        });
                        if (i + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
        }
    }
    fn import_markdown(&mut self, id: Uuid, ctx: &egui::Context) {
        #[cfg(windows)]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Markdown", &["md", "markdown", "txt"])
            .pick_file()
        {
            self.dispatch(Command::ImportMarkdown { id, path }, ctx);
        }
        #[cfg(not(windows))]
        self.dispatch(
            Command::ImportMarkdown {
                id,
                path: std::path::PathBuf::from(self.markdown_path.trim()),
            },
            ctx,
        );
    }
    fn session_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: Uuid) {
        let Some(r) = self.data.sessions.iter().find(|r| r.id == id).cloned() else {
            ui.spinner();
            return;
        };
        let busy = self.busy || r.status == "processing";
        design::eyebrow(ui, &date(&r.created_at));
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(&r.title).size(32.0).strong());
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(if r.pinned { "Unpin" } else { "Pin" }),
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
        ui.horizontal_wrapped(|ui| {
            design::pill(
                ui,
                if r.status == "processing" {
                    "PROCESSING"
                } else if r.notes.is_empty() {
                    "RECORDING SAVED"
                } else {
                    "NOTES READY"
                },
                ACCENT,
            );
            design::subtitle(
                ui,
                &format!(
                    "{} action items · {} completed",
                    r.actions.len(),
                    r.actions.iter().filter(|a| a.done).count()
                ),
            );
        });
        if let Some(error) = &r.error {
            ui.colored_label(egui::Color32::from_rgb(207, 92, 104), error);
        }
        ui.add_space(18.0);
        design::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !busy && self.tools.speech,
                        design::primary(if r.transcript.is_empty() {
                            "Create transcript"
                        } else {
                            "Refresh transcript"
                        }),
                    )
                    .clicked()
                {
                    self.dispatch(Command::Transcribe(id), ctx);
                }
                if ui
                    .add_enabled(
                        !busy && cfg!(windows) && !self.recording(),
                        egui::Button::new("Play audio"),
                    )
                    .clicked()
                {
                    self.stop_playback();
                    self.dispatch(Command::Playback(id), ctx);
                }
                #[cfg(windows)]
                if self.player.is_some() && ui.button("Stop playback").clicked() {
                    self.stop_playback();
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Import Markdown…"))
                    .clicked()
                {
                    self.import_markdown(id, ctx);
                }
                if ui.button("Export notes…").clicked() {
                    self.export(id, true, ctx);
                }
            });
            #[cfg(not(windows))]
            ui.add(
                egui::TextEdit::singleline(&mut self.markdown_path)
                    .hint_text("Returned Markdown path (preview)"),
            );
            if !self.tools.speech {
                design::subtitle(
                    ui,
                    "Speech engine is not ready. Check Settings for its status.",
                );
            }
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    design::subtitle(ui, "Working on this conversation…");
                });
            }
        });
        ui.add_space(18.0);
        ui.horizontal_wrapped(|ui| {
            for (i, label) in [
                (0, "Overview"),
                (1, "Actions"),
                (2, "Transcript"),
                (3, "Audio & AI"),
            ] {
                if nav(ui, label, self.session_tab == i).clicked() {
                    self.session_tab = i;
                }
            }
        });
        ui.add_space(12.0);
        match self.session_tab {
            0 => {
                let side = ui.available_width() > 790.0;
                if side {
                    let width = ui.available_width() - 306.0;
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 0.0),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| self.notes_panel(ui, ctx, &r, busy),
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(296.0, 0.0),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                self.session_actions(ui, ctx, &r, busy);
                            },
                        );
                    });
                } else {
                    self.notes_panel(ui, ctx, &r, busy);
                    ui.add_space(18.0);
                    self.session_actions(ui, ctx, &r, busy);
                }
            }
            1 => self.session_actions(ui, ctx, &r, busy),
            2 => {
                design::card(ui).show(ui,|ui|{ui.set_width(ui.available_width());ui.heading("Full transcript");ui.add_space(8.0);if r.transcript.is_empty(){design::subtitle(ui,"Create a transcript or import a Markdown document with a Full transcript section.");}else{egui_commonmark::CommonMarkViewer::new().show(ui,&mut self.markdown_cache,&r.transcript);ui.add_space(12.0);if ui.button("Copy transcript").clicked(){ctx.copy_text(r.transcript.clone());}
if ui.button("Export transcript…").clicked(){self.export(id,false,ctx);}}});
            }
            _ => self.audio_ai_panel(ui, ctx, &r, busy),
        }
        ui.add_space(20.0);
        egui::CollapsingHeader::new("Session options").show(ui, |ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Delete conversation…"))
                .clicked()
            {
                self.delete_confirm = Some(id);
            }
        });
    }
    fn notes_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        r: &app::Recording,
        busy: bool,
    ) {
        design::card(ui).show(ui,|ui|{
            ui.set_width(ui.available_width());
            ui.heading("Notes & decisions");ui.add_space(12.0);
            if r.notes.is_empty(){
                ui.add_space(12.0);ui.label(egui::RichText::new("From conversation to clarity.").size(21.0).strong());
                design::subtitle(ui,"Create a transcript, then let your signed-in agent turn it into notes and next steps.");
                if ui.add(design::primary("Set up AI notes")).clicked(){self.session_tab=3;}
                ui.add_space(16.0);
            }else{
                let mut document=r.notes.clone();
                let response=egui_commonmark::CommonMarkViewer::new().show_mut(ui,&mut self.markdown_cache,&mut document);
                if response.response.changed()&&!busy{self.dispatch(Command::Notes{id:r.id,notes:document},ctx);}
            }
            ui.add_space(12.0);
            egui::CollapsingHeader::new("Edit Markdown").show(ui,|ui|{
                self.notes_dirty|=ui.add(egui::TextEdit::multiline(&mut self.notes_edit).desired_width(f32::INFINITY).desired_rows(14).font(egui::TextStyle::Monospace)).changed();
                if ui.add_enabled(!busy&&self.notes_dirty,design::primary("Save changes")).clicked(){self.dispatch(Command::Notes{id:r.id,notes:self.notes_edit.clone()},ctx);self.notes_save_pending=true;}
            });
        });
    }
    fn session_actions(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        r: &app::Recording,
        busy: bool,
    ) {
        design::card(ui).show(ui,|ui|{
            ui.set_width(ui.available_width());ui.heading("Next steps");
            let total=r.actions.len();let done=r.actions.iter().filter(|a|a.done).count();
            design::subtitle(ui,&format!("{done} of {total} complete"));
            if total>0{ui.add(egui::ProgressBar::new(done as f32/total as f32).fill(ACCENT).desired_height(4.0));}
            ui.add_space(8.0);
            for a in &r.actions{
                let mut checked=a.done;
                let text=if a.done{egui::RichText::new(&a.title).strikethrough().color(design::muted(ui))}else{egui::RichText::new(&a.title)};
                if ui.add_enabled(!busy,egui::Checkbox::new(&mut checked,text)).changed(){self.dispatch(Command::Complete{id:r.id,action:a.id,done:checked},ctx);}
                if let Some(owner)=&a.owner{design::subtitle(ui,owner);}
                if let Some(due)=&a.due{design::subtitle(ui,&format!("Due {due}"));}
                let copied=self.data.actions.iter().any(|g|g.source_session==r.id&&g.source_action==a.id);
                if !a.done&&ui.add_enabled(!busy&&!copied,egui::Button::new(if copied{"Added to global"}else{"+ Add to global"}).small()).clicked(){self.dispatch(Command::Copy{id:r.id,action:a.id},ctx);}
                ui.add_space(8.0);ui.separator();ui.add_space(4.0);
            }
            if total==0{ui.add_space(12.0);design::subtitle(ui,"Action items will appear here when notes are ready. Imported Markdown checkboxes become actions too.");}
        });
    }
    fn audio_ai_panel(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        r: &app::Recording,
        busy: bool,
    ) {
        design::card(ui).show(ui,|ui|{
            ui.set_width(ui.available_width());design::eyebrow(ui,"YOUR SUBSCRIPTION");ui.heading("Turn the transcript into notes");
            design::subtitle(ui,"Whisper handles speech locally. Your chosen agent receives the transcript when you generate notes.");
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui|{
                for (key,label,installed) in [("codex","Codex / ChatGPT",self.tools.codex.installed),("claude","Claude Code",self.tools.claude.installed)]{
                    if ui.add_enabled(installed,egui::Button::new(label).selected(self.provider==key)).clicked(){self.provider=key.into();}
                }
            });
            if !self.tools.codex.installed&&!self.tools.claude.installed{design::subtitle(ui,"No subscription agent detected. Settings links to the official installers.");}
            ui.checkbox(&mut self.ai_consent,"Share this transcript with my selected agent.");
            if ui.add_enabled(!busy&&!r.transcript.is_empty()&&!self.provider.is_empty()&&self.ai_consent,design::primary("Generate notes & actions")).clicked(){self.dispatch(Command::Generate{id:r.id,provider:self.provider.clone(),consent:true},ctx);}
            if r.transcript.is_empty(){design::subtitle(ui,"Create a transcript first.");}
            ui.add_space(12.0);
            egui::CollapsingHeader::new("Agent activity").show(ui,|ui|{
                if self.output.trim().is_empty(){design::subtitle(ui,"Agent output will appear here.");}else{let mut text=self.output.as_str();ui.add(egui::TextEdit::multiline(&mut text).desired_width(f32::INFINITY).desired_rows(12).font(egui::TextStyle::Monospace));}
            });
        });
        ui.add_space(18.0);
        design::card(ui).show(ui,|ui|{
            ui.set_width(ui.available_width());ui.heading("Use another audio service");
            design::subtitle(ui,"Export the original audio and a ready-to-copy prompt. Use a service with audio support, then bring its Markdown back here.");
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!busy,egui::Button::new("Prepare audio folder")).clicked(){self.dispatch(Command::PrepareAudio(r.id),ctx);}
                if ui.button("Open audio folder").clicked(){self.open_folder(self.state.root.join(r.id.to_string()).join("audio-handoff"));}
                if ui.button("Copy audio prompt").clicked(){ctx.copy_text(crate::document::audio_prompt().into());}
            });
            design::subtitle(ui,"Codex CLI and Claude Code do not accept audio attachments directly.");
            if ui.add_enabled(!busy,egui::Button::new("Import returned Markdown…")).clicked(){self.import_markdown(r.id,ctx);}
            design::subtitle(ui,"Or save agent-output.md in this session's folder; the app imports changes automatically.");
            if ui.small_button("Open session folder").clicked(){self.open_folder(self.state.root.join(r.id.to_string()));}
        });
    }
    fn open_folder(&mut self, path: std::path::PathBuf) {
        if !path.is_dir() {
            self.notice = "Prepare the audio folder first.".into();
            return;
        }
        #[cfg(windows)]
        {
            let mut c = std::process::Command::new("explorer.exe");
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x08000000);
            if let Err(e) = c.arg(&path).spawn() {
                self.notice = format!("Cannot open folder: {e}");
            }
        }
        #[cfg(not(windows))]
        {
            self.notice = format!("Folder: {}", path.display());
        }
    }
    fn actions_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        design::eyebrow(ui, "ALL YOUR CONVERSATIONS");
        ui.label(
            egui::RichText::new("A little less on your mind.")
                .size(34.0)
                .strong(),
        );
        design::subtitle(
            ui,
            "One list for the next steps that matter. Each action keeps a link to its conversation.",
        );
        ui.add_space(24.0);
        let total = self.data.actions.len();
        let done = self.data.actions.iter().filter(|a| a.action.done).count();
        ui.horizontal_wrapped(|ui| {
            design::pill(ui, &format!("{} OPEN", total - done), ACCENT);
            design::pill(
                ui,
                &format!("{done} COMPLETE"),
                egui::Color32::from_rgb(49, 158, 125),
            );
            ui.checkbox(&mut self.hide_completed, "Hide completed");
        });
        ui.add_space(18.0);
        let actions = self.data.actions.clone();
        for g in &actions {
            if self.hide_completed && g.action.done {
                continue;
            }
            design::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    let mut checked = g.action.done;
                    let text = if checked {
                        egui::RichText::new(&g.action.title)
                            .size(17.0)
                            .strikethrough()
                            .color(design::muted(ui))
                    } else {
                        egui::RichText::new(&g.action.title).size(17.0)
                    };
                    if ui
                        .add_enabled(!self.busy, egui::Checkbox::new(&mut checked, text))
                        .changed()
                    {
                        self.dispatch(
                            Command::GlobalComplete {
                                id: g.id,
                                done: checked,
                            },
                            ctx,
                        );
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if self.data.sessions.iter().any(|r| r.id == g.source_session) {
                        if ui.link(&g.source_title).clicked() {
                            self.navigate(View::Session(g.source_session));
                        }
                    } else {
                        design::subtitle(ui, &format!("{} · archived source", g.source_title));
                    }
                    if let Some(owner) = &g.action.owner {
                        design::subtitle(ui, owner);
                    }
                    if let Some(due) = &g.action.due {
                        design::subtitle(ui, &format!("Due {due}"));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Remove").small())
                            .clicked()
                        {
                            self.dispatch(Command::GlobalRemove(g.id), ctx);
                        }
                    });
                });
            });
            ui.add_space(10.0);
        }
        if actions.is_empty() {
            design::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add_space(24.0);
                ui.heading("Your next steps, all together.");
                design::subtitle(
                    ui,
                    "Open a conversation and add its open action items to this list.",
                );
                ui.add_space(24.0);
            });
        } else if self.hide_completed && total == done {
            design::subtitle(ui, "Everything is checked off. Enjoy the clear list.");
        }
    }
    fn settings_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(
            egui::RichText::new("Make this space yours.")
                .size(34.0)
                .strong(),
        );
        design::subtitle(
            ui,
            "Appearance, speech recognition, and your connected tools.",
        );
        ui.add_space(24.0);
        design::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.heading("Appearance");
            design::subtitle(ui, "Choose your preferred light, or follow your computer.");
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                for (key, name) in [("light", "Light"), ("dark", "Dark"), ("system", "System")] {
                    if ui
                        .add_enabled(
                            !self.busy,
                            egui::Button::new(name)
                                .selected(self.theme == key)
                                .min_size(egui::vec2(120.0, 48.0)),
                        )
                        .clicked()
                    {
                        self.theme = key.into();
                        Self::theme(ctx, key);
                        self.dispatch(Command::Theme(key.into()), ctx);
                    }
                }
            });
        });
        ui.add_space(18.0);
        design::card(ui).show(ui,|ui|{
            ui.set_width(ui.available_width());ui.heading("Speech recognition");
            design::pill(ui,if !self.tools.checked{"CHECKING"}else if self.tools.speech{"WHISPER READY"}else{"NOT READY"},ACCENT);
            design::subtitle(ui,"Speech is processed on your computer. Set Bulgarian explicitly for Bulgarian conversations.");
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui|{
                for (key,name) in [("bg","Bulgarian"),("en","English"),("auto","Auto-detect")]{
                    if ui.add_enabled(!self.busy,egui::Button::new(name).selected(self.speech_language==key)).clicked(){self.speech_language=key.into();self.save_speech_settings(ctx);}
                }
            });
            design::subtitle(ui,"The bundled tiny model favors speed. A multilingual small or medium model can improve Bulgarian accuracy.");
            #[cfg(windows)] if ui.button("Choose a Whisper model…").clicked() && let Some(path)=rfd::FileDialog::new().add_filter("Whisper model",&["bin"]).pick_file(){self.speech_model=path.display().to_string();self.save_speech_settings(ctx);}
            if !self.speech_model.is_empty(){design::subtitle(ui,&self.speech_model);if ui.button("Use bundled model").clicked(){self.speech_model.clear();self.save_speech_settings(ctx);}}
            if ui.link("Download multilingual models").clicked(){ctx.open_url(egui::OpenUrl::new_tab("https://huggingface.co/ggerganov/whisper.cpp/tree/main"));}
        });
        ui.add_space(18.0);
        design::card(ui).show(ui,|ui|{
            ui.set_width(ui.available_width());ui.heading("Subscription agents");design::subtitle(ui,"Connect an installed agent using its own subscription login. No API keys are required here.");ui.add_space(10.0);
            for (name,info,url) in [("Codex / ChatGPT",&self.tools.codex,"https://developers.openai.com/codex/cli/"),("Claude Code",&self.tools.claude,"https://code.claude.com/docs/en/setup")]{
                ui.horizontal_wrapped(|ui|{ui.strong(name);design::pill(ui,if info.installed{"DETECTED"}else{"NOT INSTALLED"},if info.installed{egui::Color32::from_rgb(49,158,125)}else{design::muted(ui)});if ui.link("Official setup guide").clicked(){ctx.open_url(egui::OpenUrl::new_tab(url));}});
                if let Some(version)=&info.version{design::subtitle(ui,version);}
                ui.add_space(8.0);
            }
            if ui.button("Check tools again").clicked(){self.check_tools(ctx);}
        });
        ui.add_space(18.0);
        design::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.heading("Your data");
            design::subtitle(
                ui,
                "Recordings, Markdown, and action lists stay in your local workspace.",
            );
            ui.label(self.state.root.display().to_string());
            if ui.button("Open data folder").clicked() {
                self.open_folder(self.state.root.clone());
            }
        });
    }
    fn save_speech_settings(&mut self, ctx: &egui::Context) {
        self.dispatch(
            Command::SpeechSettings {
                language: self.speech_language.clone(),
                model: self.speech_model.clone(),
            },
            ctx,
        );
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
fn nav(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let fill = if selected {
        if ui.visuals().dark_mode {
            egui::Color32::from_rgb(47, 39, 75)
        } else {
            egui::Color32::from_rgb(230, 224, 251)
        }
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(text)
                .color(if selected {
                    if ui.visuals().dark_mode {
                        egui::Color32::from_rgb(185, 168, 255)
                    } else {
                        ACCENT
                    }
                } else {
                    ui.visuals().text_color()
                })
                .strong(),
        )
        .fill(fill)
        .stroke(egui::Stroke::NONE)
        .corner_radius(9),
    )
}
fn short(text: &str, max: usize) -> String {
    let s: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        format!("{s}…")
    } else {
        s
    }
}
fn date(text: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%b %d, %Y")
                .to_string()
        })
        .unwrap_or_else(|_| text.to_owned())
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
