# OpenAIRecorder — native Windows desktop app

OpenAIRecorder 0.2 uses a Rust-rendered desktop window (egui/eframe), native Windows WASAPI audio capture, and a movable, always-on-top recording widget. Double-click `openairecorder.exe` to open the desktop app. The Windows startup path does not launch a browser, use a WebView, or start a localhost server.

[Download the native Windows x64 build](https://github.com/genchev/openairecorder/raw/refs/heads/main/downloads/OpenAIRecorder-Native-Windows-x64.zip). Extract the whole ZIP and follow `START-HERE.txt`. The ZIP includes Whisper, its multilingual speech model, and required app runtime DLLs. Install ffmpeg and the Visual C++ runtime as described below. No Rust compiler is needed to use the ZIP.

## Features

- Capture audio from Teams, Slack, Zoom, or any application using your Windows default playback device, plus an optional microphone.
- Pause/resume without ending the recording: both the main window and floating widget have controls. Paused time is excluded from the audio and timer; resuming appends to the same WAV. Stop saves the session, including when paused.
- Recording and the floating controls continue when the main window is minimized. No video is captured.
- Import audio/video, play recordings within the app, and export Markdown.
- Local multilingual transcription with Whisper. Raw audio stays on your computer.
- Detect installed Codex/Claude Code CLIs, show installation/sign-in guidance, and display their output/diagnostics.
- Generate summaries, notes, decisions, open questions, and action items using subscription CLI sign-in.
- Search session titles, transcripts, notes, and action text. Pin sessions above others.
- Session checkboxes; copy actions into a global checklist with independent completion. Copied actions retain their source name and survive session deletion.
- Light, Dark, and System preferences saved locally. System follows live OS appearance changes.
- Save-on-close for active recordings and recovery of readable interrupted capture WAVs at next startup.

## Install and launch on Windows 10/11 x64

1. Extract the entire native ZIP to a writable folder. Keep `libunwind.dll` and `.local-tools` with the executable.
2. Install the audio/transcription prerequisites in PowerShell:

```powershell
winget install --id Gyan.FFmpeg --exact
winget install --id Microsoft.VCRedist.2015+.x64 --exact
```

3. Restart the app from a fresh PowerShell, or sign out/in if Explorer has not received the updated PATH. Double-click `openairecorder.exe` to launch. The GUI will open as a desktop window. Settings shows which tools are detected.
4. Install at least one subscription agent and sign in:

```powershell
# Codex's npm installer requires Node.js.
npm install -g @openai/codex
codex login

# Or Claude Code:
winget install --id Anthropic.ClaudeCode --exact
claude auth login
```

See the [Codex CLI guide](https://developers.openai.com/codex/cli/) and [Claude Code setup guide](https://code.claude.com/docs/en/setup). These sign-in commands may use a browser for authentication; the recorder's GUI is native. Recent CLI versions supporting the isolation flags are required. Restart the recorder after installing tools, then use Settings → Recheck installed tools.

For source builds, install [Rust](https://www.rust-lang.org/tools/install) and its Visual Studio C++ build-tool prerequisites. Run `.\scripts\setup-windows.ps1`, `cargo build --locked --release`, then `.\scripts\start-windows.ps1`. The setup script verifies the pinned Whisper archive and tiny model's SHA-256 hashes and preserves TLS verification.

## Record a conversation

Name the conversation and choose meeting/system audio, microphone, or both. Inform participants and obtain their permission before starting. Ensure your meeting application plays through the Windows default output device; use headphones to reduce echo. Click Start recording. The small floating widget appears above other windows and can be dragged by its upper area.

Click Pause to omit a portion of the conversation, then Resume to continue the same recording. The native audio worker handles Pause/Resume/Stop independently of the GUI's rendering, so the widget still works when the main window is minimized. Stop finalizes and saves the recording. Closing the main app during recording prompts to stop and save; minimizing keeps it running.

Open the saved session and generate the local transcript. Select Codex or Claude Code, confirm sharing the transcript, and generate notes/actions. Expand the agent-output panel for progress and failures. Check actions off in the session or copy them into Global actions. Edit/save notes or export them to Markdown.

## Accounts and privacy

The app has no paid AI API integration. It removes common API-key variables from agent invocations and verifies subscription-style CLI authentication; API-key and third-party-provider authentication is rejected. Access and usage limits depend on your account's plan and account settings. Only transcript text is sent when you explicitly request agent notes.

Codex runs with read-only sandboxing, shell tools disabled, and user configuration disabled. Claude runs in safe mode with tools/MCP/customizations disabled. Authentication is retained. The app does not copy account credentials, bypass permission prompts, or sign in on your behalf. In this cloud, the existing Codex token returned HTTP 401, so live provider generation still requires testing with your usable local subscription login.

Recordings and settings are normally stored under `%LOCALAPPDATA%\OpenAIRecorder\recordings`. If a `recordings` directory from the older portable version exists next to the executable, it is reused to preserve sessions, themes, and global actions. `RECORDER_DATA_DIR` overrides the location. Files are not encrypted; use a protected local drive. Deleting a session removes its audio and logs, while copied global actions remain.

## Configuration

| Variable | Purpose |
| --- | --- |
| `RECORDER_DATA_DIR` | Override the data directory |
| `WHISPER_BIN` | Override the Whisper executable |
| `WHISPER_MODEL` | Override the GGML speech model |
| `FFMPEG_BIN` | Override ffmpeg (otherwise found on PATH) |
| `CODEX_BIN` / `CLAUDE_BIN` | Override agent executable paths |

Bundled Whisper/model paths are resolved relative to the executable, so launching from another working directory works. Tiny is a fast initial model; a base/small model can improve recognition accuracy. Native captures are mono PCM WAV, approximately 115 MB per active hour, with a safety stop before 2 GiB. Paused time consumes no WAV samples. External imports remain limited to 150 MB.

Each session directory contains original audio, `record.json`, `transcript.md`, `notes.md`, and agent logs. Global actions and theme settings live in the data root. Unreadable interrupted capture files are retained for manual recovery instead of being deleted.

## Development and validation

```bash
cargo build --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Windows starts the native GUI by default. `--desktop` opens the shared native GUI on Linux for development; Windows audio/file-picker/playback implementations are platform-specific. `--web` explicitly starts the legacy localhost development interface; on Linux this remains the default when no mode is supplied. The desktop GUI calls shared Rust services directly and needs no HTTP listener.

`scripts/setup-cloud.sh` prepares the existing isolated cloud checkout. `scripts/build-windows-cloud.sh` cross-builds the native Windows executable with checksum-verified LLVM-MinGW and packages the model/runtime DLLs. No worktrees are created. The Windows executable uses the GUI subsystem, and transcription/agent child processes do not open console windows.

Rust tests exercise storage, direct desktop commands, audio alignment/resampling, pause timing, subscription CLI contracts, and native GUI rendering/theme switching without consuming provider quota. The legacy Chromium check uses mock recognition/agents:

```bash
npm install --prefix /tmp/recorder-test-tools playwright@1.64.1 --ignore-scripts
NODE_PATH=/tmp/recorder-test-tools/node_modules node scripts/browser-smoke.cjs
```

It needs Chromium and a free port 8787. See [VALIDATION.md](VALIDATION.md) for observed outcomes and remaining checks.

## Current limits

Physical Windows capture, the floating widget, and the Windows executable need local Windows testing; cross-compilation and Linux GUI checks cannot prove hardware behavior. Capture uses the default Windows output device rather than a per-application bot/account integration. Other audio on that device can be recorded. Device changes can stop capture; the existing audio is saved when possible. Speaker identification and automatic long-transcript chunking for agents are not included. The GUI supports the generated Markdown's basic headings and lists, plus raw editing/export. Subscription login and quota are required for live agent notes.
