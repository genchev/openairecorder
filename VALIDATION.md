# Validation — Gemini desktop 0.4

- 43 Rust tests passed. New coverage includes exact 60-second segmentation and final tails, automatic recovery of an interrupted full minute and partial tail, one-/two-model Gemini request contracts, Bulgarian transcript/notes persistence, required separate-notes selection after a capability failure, auth-error retention and explicit retry, idempotent persisted checkpoints, preserved user checkboxes/action identity, and key-file separation/removal. Test requests use an isolated local HTTP fixture with a fake key, not Google's service.
- WebRTC speech-indicator buffering and silence were tested. These checks do not establish VAD accuracy for real microphones/noise.
- Headless native panels/theme checks passed. The Gemini settings and compact-controls design preview were visually inspected in a Linux virtual display, together with sample Bulgarian notes/actions. The preview is not the actual Win32 popup.
- Linux and Windows Clippy with warnings denied passed, and the Windows x64 GUI-subsystem release cross-build passed. The cross-build includes HTTPS/rustls, DPAPI bindings, native WASAPI capture/playback, local WebRTC VAD, and DPI-aware compact controls. Archive contents/checksum and runtime dependencies were checked. Whisper, speech models, CLI agents, and end-user setup scripts are not included.
- No usable Gemini API key was supplied to this cloud. Actual Google authentication, selected-model audio/structured-output support, quotas, latency, and Bulgarian recognition quality are unverified. The app's Test audio & notes control performs that provider check on the user's computer.
- Physical Windows audio, DPAPI protection/unlocking, topmost dragging, DPI changes, control interaction, and the voice indicator have not run on Windows. Linux rendering, mock API tests, and cross-compilation do not prove that behavior.

## Library and recorder follow-up

The headless UI test now verifies that an updated library snapshot preserves unsaved notes in an open session, and clears its editor only after that session is successfully removed. Existing backend tests cover deletion and survival of copied global actions. Rounded Win32 owner-drawn buttons now clear their full rectangles, and keyboard focus uses a custom purple outline. Settings preview icons use matching vector geometry. This painting fix is cross-compiled, not physically verified on Windows.

Current Gemini model IDs, audio-input pricing, and the restricted availability of 2.5 were checked against Google's model catalog and pricing documentation on 2026-10-14. Suggested models are not proof of account access or recognition quality; account discovery and the audio/notes probe remain necessary. No live Gemini calls were made.

## Compact library and 30-day Archives

27 tests pass, including archive retention just before and exactly at 30 days, repeated archive idempotence, restore canceling expiry, removal of the complete session folder, survival of copied global actions, legacy metadata without an archive field, invalid/future dates, and active-recording protection. A headless layout regression renders 25 rows and asserts fixed horizontal position, row height, and sidebar width.

The compact native library was inspected in a Linux virtual display with isolated Bulgarian/English sample records. Pinning into the separate section, archive confirmation, archiving into the expandable folder, and restoration were exercised using pointer events. This is native Linux UI verification; physical Windows UI behavior remains unverified.

## Application icon (earlier footer branding removed)

The Windows release PE was inspected with llvm-readobj: it contains seven ICON resources and one GROUP_ICON resource. The ICO sizes (16, 24, 32, 48, 64, 128, 256) and the runtime 64×64 RGBA length were verified. Eframe receives the logo through the native viewport icon setting, which supplies its Windows title/taskbar icon setter. The footer link and its placement below Saved on this computer were visually inspected in the Linux native display. 27 tests, Linux/Windows Clippy with warnings denied, and the Windows release build passed. Actual Windows taskbar/Explorer appearance remains unverified on a physical Windows system.

## Global task management

33 tests pass. New coverage checks personal task creation, exact date and title validation, persisted edits, clearing due dates, concurrent pin/completion/content updates without lost fields, stable added date and action/source identities, legacy metadata, independently edited meeting copies and copy deduplication, freeform-deadline preservation, Unicode search, all four sort modes, undated-task ordering, priority/completion behavior, year-boundary date rollover, and leap-day validity. A headless calendar regression verifies that October 1, 2026 and February 1, 2028 align with Thursday and Tuesday.

The native Linux GUI was exercised with isolated sample tasks: creating a personal task, searching for it, pinning it, editing its due date using the calendar, saving, and reading the persisted JSON. Dark and Light layouts, urgent labels/borders, and the calendar were visually inspected. Native restart loaded saved tasks and pins. Windows Clippy with warnings denied and the Windows release cross-build passed. Physical Windows interaction remains unverified.

## Workspace relocation and README-only credit

39 tests pass, including byte-preserving relocation of audio, pending segments, Unicode notes and a saved fixture key; private key-file permissions on Unix; multiple moves and returning to the original folder; a fixed locator outside the workspace; destination collision/nesting rejection; rollback after a forced locator-commit failure; missing/invalid saved locations; and rejection of linked entries. Tests use isolated temporary folders, with no real Gemini credentials.

The Linux native GUI moved an isolated workspace containing four sessions and eight tasks, left only the locator at the original development anchor, then saved a new task in the new location. Restarting with that original anchor loaded the relocated records and tasks. Settings displays the new root. The Windows folder-picker branch and release are cross-compiled; physical Windows folder selection, cross-volume behavior, and DPAPI loading after relocation remain unverified.

The website link was removed from the application footer. Development credit appears at the end of README.md. Relevant screenshots were refreshed to show the current footer.

## Actions from note points

42 tests pass. New coverage includes target-heading extraction, formatted links/code/Bulgarian, non-overlapping Markdown offsets, ignoring unrelated sections and fenced examples, literal Markdown task escaping, reuse of the existing Action items section, idempotent creation, preserved original notes/completion, source-point validation, copy to Global Actions, and stable promotion identity after action rewording. A defaulted source-point/action mapping preserves compatibility with old session metadata.

The native Linux GUI was exercised using isolated notes: points in Key Points, Decisions and Open Questions created session actions, the first action was copied to Global Actions, and its generated Markdown checkbox updated the structured action's completion. The checkmark state and source formatting were visually inspected. Windows cross-compilation/static checks passed; physical Windows interaction remains unverified.

Rendered notes now hide action checklist sections already represented in the session action panel, while preserving the complete source Markdown for editing/export. The new test covers following sections, unknown tasks that must remain visible, prose-only sections, Bulgarian headings, and fenced examples.
