# OpenAIRecorder

A Rust meeting recorder for Windows, with a local browser GUI and a native, movable, always-on-top recording widget. Record audio from Teams, Slack, Zoom, browser meetings, or a microphone. Transcribe locally with Whisper, then run your signed-in Codex or Claude Code CLI to create notes and action items.

No AI API integration or API keys are required. Agent access uses your CLI account and its plan's limits. ChatGPT subscription access to Codex and Claude subscription access to Claude Code depend on the plan. The app verifies subscription-style authentication and rejects API-key authentication before generating notes. A website handoff is also available.

## Download the Windows test build

[Download the Windows x64 ZIP](https://github.com/genchev/openairecorder/raw/refs/heads/main/downloads/OpenAIRecorder-Windows-x64.zip). Extract the entire folder and follow `START-HERE.txt`. The archive includes the speech model and Whisper runtime; ffmpeg, the Microsoft Visual C++ runtime, and a signed-in agent are installed separately as described there.

The SHA-256 checksum is in [downloads/OpenAIRecorder-Windows-x64.zip.sha256](downloads/OpenAIRecorder-Windows-x64.zip.sha256). This is a cross-built prototype; native Windows behavior still needs local testing.

## Features

- Screen/tab audio capture plus optional microphone, or microphone-only recording.
- Draggable native Windows recording widget, elapsed timer, and Stop button.
- Audio/video imports up to 150 MB, playback, and audio downloads.
- Local multilingual transcription via whisper.cpp; no audio sent to the agent.
- Automatic CLI detection, installation guidance, agent progress and diagnostics.
- Agent-generated Markdown summaries, notes, decisions, and open questions.
- Session checklists with owner/deadline fields when stated in the meeting.
- Session search across titles, transcripts, notes, and action text; pin sessions above others.
- Copy actions into a persistent global checklist. Its completion is independent of session completion; copied actions survive session deletion.
- Local Markdown files and exports. Interrupted jobs can be retried.
- Persistent Light, Dark, and System themes in Settings; System follows live OS appearance changes.

## Windows setup

Install [Rust](https://www.rust-lang.org/tools/install), including the Visual Studio C++ build tools requested by the installer. Install a recent Chrome or Edge. From PowerShell:

```powershell
winget install --id Gyan.FFmpeg --exact
winget install --id Microsoft.VCRedist.2015+.x64 --exact
# Open a new PowerShell so it sees the updated PATH.
cd path\to\openairecorder
.\scripts\setup-windows.ps1
cargo build --locked --release
.\scripts\start-windows.ps1
```

The setup script downloads the official whisper.cpp v1.8.7 Windows binaries and the multilingual tiny model. Downloads use TLS verification, with pinned SHA-256 checks for the Windows Whisper archive and speech model. Tiny is a fast starting model; for better accuracy use a base or small model from the [official model repository](https://huggingface.co/ggerganov/whisper.cpp) and set `WHISPER_MODEL` to its path. Recognition quality depends on sound quality, language, and model size. This version does not reliably identify individual speakers.

The server listens only on `127.0.0.1:8787`. On Windows it opens the GUI in an Edge/Chrome app window when available. You can also enter `http://127.0.0.1:8787` in Chrome or Edge manually. Keep the server running during use.

Install at least one agent and sign in with your subscription:

```powershell
# Codex requires Node.js for the npm installer.
npm install -g @openai/codex
codex login

# Or install Claude Code (see the official setup guide for prerequisites).
winget install --id Anthropic.ClaudeCode --exact
claude auth login
```

See the [Codex CLI guide](https://developers.openai.com/codex/cli/) and [Claude Code setup guide](https://code.claude.com/docs/en/setup). After installing an agent, restart the server to refresh its PATH. The GUI has a Recheck button. CLI detection is not a guarantee of login or remaining quota; errors are shown when the agent runs. Native CLI paths and npm JavaScript launchers are supported on Windows without putting meeting text through a shell.

## Recording a meeting

1. Inform participants and obtain their permission. Name the conversation.
2. Choose Meeting audio and, optionally, Include my microphone. Use headphones to reduce duplicated speech and echo.
3. In Chrome/Edge's sharing dialog select a screen or browser tab and enable **Share audio**. For desktop Teams/Zoom/Slack on Windows, choose a screen with system audio sharing. Browser support and organization policies can limit capture.
4. The native Windows widget appears after recording begins. Drag its upper area to move it; click Stop recording to stop and save. The GUI also has Stop controls.
5. Keep the GUI tab/window and server open while recording. Switching/minimizing is supported by the capture mechanism; closing the GUI or stopping browser sharing ends capture. No screen video is saved. This is user-started audio capture, not automatic platform-bot integration.
6. Generate the local transcript. Select the installed CLI, confirm sharing the transcript, and generate notes. Follow agent output in the expandable panel.
7. Check actions off in the session or copy them into Global actions. Edit/save notes as needed.

Raw audio is local. Transcript text is sent to the selected CLI's provider when you explicitly generate notes. The CLI is run in a session directory, with read-only sandboxing and shell tools disabled for Codex, and tools disabled for Claude. User/project customizations are disabled for note-generation runs, while subscription sign-in is retained. Recent CLI versions supporting these isolation flags are required. The app removes common API-key environment variables and checks CLI authentication. It does not bypass permission prompts or sign in on your behalf. Your own CLI/account policies still apply.

## Development and configuration

```bash
cargo build --locked
cargo test --locked
cargo clippy --locked -- -D warnings
cargo run --locked
```

Rust is the backend and Windows widget; HTML/CSS/JavaScript provide the local GUI. No frontend build or CDN is required. On Linux/macOS, imports and microphone/browser-supported audio capture work, but the native topmost widget is Windows-only. The browser fallback widget stays within the GUI.

| Variable | Purpose |
| --- | --- |
| `RECORDER_DATA_DIR` | Local data directory (default `recordings` relative to working directory) |
| `WHISPER_BIN` | Path to `whisper-cli` / `whisper-cli.exe` |
| `WHISPER_MODEL` | Path to a whisper.cpp GGML model |
| `FFMPEG_BIN` | ffmpeg executable (default `ffmpeg` on PATH) |
| `CODEX_BIN` | Codex executable or supported npm launcher path |
| `CLAUDE_BIN` | Claude executable or supported npm launcher path |
| `RECORDER_NO_BROWSER` | Set to suppress automatic GUI launch on Windows |

Each session directory contains original audio, `record.json`, `transcript.md`, `notes.md`, Whisper text/subtitles, and agent output logs. Global actions are in `recordings/global-actions.json`. These files are not encrypted; keep the directory on a protected local drive. Deleting a session deletes its audio and logs; copied global actions remain. Do not put recordings or account credentials in Git.

### Cloud environment

`scripts/setup-cloud.sh` installs the Rust toolchain and whisper.cpp, downloads the model, and builds/tests the app. It uses the existing isolated checkout, not worktrees. Model downloads require `huggingface.co` and its delivery domains. A Linux cloud environment cannot validate Windows audio capture, the widget's native appearance, or your personal subscription. Unit and integration tests use mock agent processes so they do not consume subscription quota.

## Current limits

The GUI must stay open during recording, and an installed/local speech model is required for transcription. Long meetings must fit your agent's context and subscription limits; automatic transcript chunking for agent notes is not yet implemented. Native Teams/Slack/Zoom account integrations, background recording independent of the GUI, automatic speaker identification, and a packaged Windows installer are not included in this initial version.

### Browser workflow test (Linux cloud)

The existing Rust tests cover persistence, authentication contracts, origin protection, pins, action completion/copying, and theme settings. The optional Chromium workflow test exercises the real HTTP/UI/ffmpeg flow with mock recognizer/agent executables and a simulated microphone, without subscription usage:

```bash
npm install --prefix /tmp/recorder-test-tools playwright@1.64.1 --ignore-scripts
cargo build --locked
NODE_PATH=/tmp/recorder-test-tools/node_modules node scripts/browser-smoke.cjs
```

It requires `/usr/bin/chromium` (or set `CHROMIUM_BIN`), ffmpeg, and a free port 8787. It creates and cleans up its own temporary data directory and server. It tests Light/Dark persistence and live System theme changes. Mock recognition and agents do not validate real speech accuracy or live provider access.

### Windows test package

A Windows x64 test package can be built from this cloud checkout with `bash scripts/build-windows-cloud.sh`. The script verifies the compiler and Whisper runtime archives against their official release SHA-256 digests. It packages the app, required third-party runtime DLLs, license notices, Windows setup scripts, and the speech model when installed. The resulting ZIP is in `dist/`. The cross-built executable has not been run on a Windows machine; use `START-HERE.txt` in the ZIP to try it locally. No Rust compiler is required to use the ZIP.
