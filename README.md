# OpenAIRecorder — Gemini meeting workspace

OpenAIRecorder 0.4 is a native Rust Windows app that sends recorded audio directly to Gemini in one-minute batches. It fills the transcript, cumulative Markdown notes, decisions, and action items as responses arrive. Whisper and subscription CLI agents have been removed.

[Gemini settings preview](docs/screenshots/gemini-settings.png) · [Light workspace](docs/screenshots/conversation-light.png) · [Dark workspace](docs/screenshots/conversation-dark.png). These are native Linux screenshots with sample meeting data, not live Google recognition results.

[Download the Gemini Windows x64 app](https://github.com/genchev/openairecorder/raw/refs/heads/main/downloads/OpenAIRecorder-Gemini-Windows-x64.zip)

Extract the ZIP and double-click `openairecorder.exe`. Keep `libunwind.dll` beside it. No setup scripts, Rust, local speech models, ffmpeg, or Visual C++ installer are required.

## Gemini settings

Open Settings, enter your **Gemini API key**, and refresh the available models. Choose an audio-capable model from your account's list, or enter a custom model ID. By default the same model transcribes audio and returns structured Markdown notes and actions.

**Test audio & notes** verifies the selected configuration with one second of generated silence. This is an API request; if combined notes fail but audio transcription works, the app enables and requires a separate notes-model choice. You can also choose separate models yourself: the audio model transcribes the segment and the notes model processes its text and meeting context. Save settings before recording.

Language detection is automatic, including Bulgarian. Prompts preserve the spoken language, mark unclear speech, and discourage invented speakers, names, owners, dates, and commitments. Recognition and summarization quality still depend on the chosen Google model and audio quality.

Gemini API availability, quota, and pricing apply separately from ChatGPT/Claude subscriptions. Model listing confirms availability, not audio support; use the capability test. [Google AI Studio keys](https://aistudio.google.com/apikey), [audio documentation](https://ai.google.dev/gemini-api/docs/audio), and [structured output documentation](https://ai.google.dev/gemini-api/docs/structured-output).

On Windows the key is encrypted with **DPAPI for the current Windows user** and stored separately in `gemini-key.bin`. It is not placed in recordings, Markdown exports, settings JSON, provider URLs, or request logs. Saving an empty key removes the stored key. The Linux development preview uses an owner-only key file rather than Windows encryption. Do not share that file.

## Recording and incremental notes

1. Name the conversation, select meeting/system audio and/or microphone, and confirm that participants know audio is being recorded and sent to Gemini.
2. Start recording. The conversation opens immediately, so you can watch its notes, transcript, and next steps fill in.
3. Each **60 seconds of recorded audio** produces a standalone WAV and queues a Gemini request. Capture continues while requests are processed sequentially. After each response, the transcript appends that segment and cumulative notes/actions update. This is minute-batch processing, **not word-by-word streaming**; response latency and backlogs are visible in the session.
4. Pause omits new audio and timer time without ending the session. Previously queued audio can still finish processing. Resume continues the same recording.
5. Stop saves the full recording and also queues the final partial minute. Leave the app open to finish processing, or close it and resume pending work on the next launch.

Capture uses the Windows default playback device, so Teams, Slack, Zoom, and other apps should play through that device. Other sound on that output can be recorded too. Use headphones to reduce echo. No video is captured.

Each minute uses approximately 1.92 MB of PCM16 WAV before Base64 encoding, below Gemini's 20 MB inline-request limit. Native audio is mono 16 kHz PCM16 WAV, about 115 MB per active hour, with a safety stop before 2 GiB.

## Discreet floating controls

![Compact recording-controls design preview](docs/screenshots/recording-controls-preview.png)

The screenshot shows the clearly labeled Settings preview; the actual Win32 popup still needs Windows testing.

The movable, always-on-top recorder is a **234 × 52 logical-pixel strip**, scaled for your monitor. It has a recording/paused dot, timer, small pause/resume and stop buttons, and three speech-activity bars. It has no title bar or large form controls.

A local **WebRTC voice-activity detector** drives the bars, with a short hold to avoid flicker. It does not wait for Gemini, transcribe words, or identify people. Like other VAD systems it can misclassify noise or quiet speech. Paused recording has no speech indicator. The strip continues to work with the main window minimized. Settings includes a clearly labeled design preview; it is not a live microphone test.

## Notes and actions

The workspace keeps modern Light, Dark, and System themes, Cyrillic typography, session search, pins, formatted CommonMark notes, a full-transcript tab, and a next-steps panel.

Action updates match existing IDs or normalized titles. User completion flags and known metadata are preserved. Check actions off in the session or add open actions to Global actions. Global copies have independent completion and retain their source even after deletion. Notes can be edited after recording, exported, or imported from UTF-8 Markdown; explicit task checkboxes become actions. Ordinary bullets do not become invented tasks. Imported Markdown images are not fetched.

Gemini is asked to return the current segment's transcript only, plus cumulative notes and explicit commitments. Empty transcripts do not add invented notes/actions. User checkboxes remain authoritative. Successful responses are cached before committing the checkpoint; replaying a saved response does not append the transcript twice. A crash before a provider response is cached can still cause a repeat API request and possible repeat charges.

## Errors and recovery

Quota/rate-limit, transient server, and connection failures get bounded retries. Authentication, unavailable-model, unsupported-output, and other errors retain pending audio and show an actionable message. Update settings and choose **Retry pending audio**. Missing keys stop uploads; adding a key lets you explicitly resume processing. Successful processed chunk files are removed after commit, while the full original audio remains.

Interrupted native recordings recover readable WAV audio and reconstruct any final unqueued partial minute. The app also retains readable legacy portable recordings. Unsupported or corrupt audio cannot be guaranteed recoverable.

Existing/imported **mono 16 kHz PCM16 WAV** can be processed in one-minute batches. External imports remain limited to 150 MB. Other formats from older versions are preserved, but currently cannot be decoded for Gemini processing inside the app.

## Storage and privacy

Audio, text, and meeting context are sent to Google when you record/process with Gemini. The app does not copy account credentials or use browser sessions. Recordings and notes themselves are **not encrypted**; use a protected local drive.

Data normally lives in `%LOCALAPPDATA%\OpenAIRecorder\recordings`. An existing portable `recordings` folder next to the executable is reused. `RECORDER_DATA_DIR` overrides it. A session contains original `audio`, pending minute WAVs under `chunks`, `record.json`, `transcript.md`, `notes.md`, `live.json` (models/progress without a key), and cached Gemini results. `gemini-settings.json` stores model choices without credentials. Themes and global actions remain in the data root. Deleting a session removes its files; copied global actions remain.

## Development and validation

```bash
cargo build --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The app defaults to its native GUI. Linux `--desktop` previews the same interface with an X display; capture, playback, DPAPI, and the real floating strip are Windows-specific. Explicit `--web` exposes a loopback development API only, not a browser app. Cloud developer setup/build scripts are not distributed as end-user installers.

Tests use a local simulated Gemini API and do not consume provider quota. See [VALIDATION.md](VALIDATION.md). Live Google authentication, model accuracy, quotas, and physical Windows capture/DPAPI/widget behavior need testing on Windows with your own key entered in the app.
