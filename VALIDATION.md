# Validation — Gemini desktop 0.4

- 25 Rust tests passed. New coverage includes exact 60-second segmentation and final tails, automatic recovery of an interrupted full minute and partial tail, one-/two-model Gemini request contracts, Bulgarian transcript/notes persistence, required separate-notes selection after a capability failure, auth-error retention and explicit retry, idempotent persisted checkpoints, preserved user checkboxes/action identity, and key-file separation/removal. Test requests use an isolated local HTTP fixture with a fake key, not Google's service.
- WebRTC speech-indicator buffering and silence were tested. These checks do not establish VAD accuracy for real microphones/noise.
- Headless native panels/theme checks passed. The Gemini settings and compact-controls design preview were visually inspected in a Linux virtual display, together with sample Bulgarian notes/actions. The preview is not the actual Win32 popup.
- Linux and Windows Clippy with warnings denied passed, and the Windows x64 GUI-subsystem release cross-build passed. The cross-build includes HTTPS/rustls, DPAPI bindings, native WASAPI capture/playback, local WebRTC VAD, and DPI-aware compact controls. Archive contents/checksum and runtime dependencies were checked. Whisper, speech models, CLI agents, and end-user setup scripts are not included.
- No usable Gemini API key was supplied to this cloud. Actual Google authentication, selected-model audio/structured-output support, quotas, latency, and Bulgarian recognition quality are unverified. The app's Test audio & notes control performs that provider check on the user's computer.
- Physical Windows audio, DPAPI protection/unlocking, topmost dragging, DPI changes, control interaction, and the voice indicator have not run on Windows. Linux rendering, mock API tests, and cross-compilation do not prove that behavior.

## Library and recorder follow-up

The headless UI test now verifies that an updated library snapshot preserves unsaved notes in an open session, and clears its editor only after that session is successfully removed. Existing backend tests cover deletion and survival of copied global actions. Rounded Win32 owner-drawn buttons now clear their full rectangles, and keyboard focus uses a custom purple outline. Settings preview icons use matching vector geometry. This painting fix is cross-compiled, not physically verified on Windows.

Current Gemini model IDs, audio-input pricing, and the restricted availability of 2.5 were checked against Google's model catalog and pricing documentation on 2026-10-14. Suggested models are not proof of account access or recognition quality; account discovery and the audio/notes probe remain necessary. No live Gemini calls were made.
