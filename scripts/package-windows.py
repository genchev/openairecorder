"""Package a cross-built Windows app and verified third-party runtimes."""
import hashlib
from pathlib import Path
import zipfile

repo = Path(__file__).resolve().parent.parent
tools = Path('/workspace/.tools')
compiler = tools / 'llvm-mingw-20261006-ucrt-ubuntu-22.04-x86_64'
assert hashlib.sha256((tools / 'llvm-mingw.tar.xz').read_bytes()).hexdigest() == '5f9c6ed95b2d4bdb2869a488c5fd5857fbdabcf288a0aa3eb1da43f6a08d8ab4'
output = repo / 'dist' / 'OpenAIRecorder-Gemini-Windows-x64.zip'
output.parent.mkdir(exist_ok=True)
prefix = 'OpenAIRecorder'
with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED, strict_timestamps=False) as archive:
    archive.write(repo / 'target/x86_64-pc-windows-gnullvm/release/openairecorder.exe', f'{prefix}/openairecorder.exe')
    archive.write(compiler / 'x86_64-w64-mingw32/bin/libunwind.dll', f'{prefix}/libunwind.dll')
    archive.write(tools / 'llvm-LICENSE.txt', f'{prefix}/licenses/LLVM.txt')
    archive.write(compiler / 'x86_64-w64-mingw32/share/mingw32/COPYING', f'{prefix}/licenses/mingw-w64.txt')
    for name in ['README.md', 'VALIDATION.md', 'docs/screenshots/conversation-dark.png', 'docs/screenshots/conversation-light.png', 'docs/screenshots/gemini-settings.png', 'docs/screenshots/recording-controls-preview.png', 'docs/screenshots/library-archives.png', 'docs/screenshots/global-actions-dark.png', 'docs/screenshots/global-actions-light.png', 'docs/screenshots/global-task-editor.png', 'docs/screenshots/workspace-settings.png']:
        archive.write(repo / name, f'{prefix}/{name}')
    archive.write(repo / 'assets/fonts/OFL.txt', f'{prefix}/licenses/Inter-OFL.txt')
    registry = tools / 'cargo/registry/src'
    import tomllib
    dependencies = {(entry['name'], entry['version']) for entry in tomllib.loads((repo / 'Cargo.lock').read_text())['package']}
    for index in registry.iterdir():
        if not index.is_dir(): continue
        for crate in index.iterdir():
            if not any(crate.name == f'{name}-{version}' for name, version in dependencies): continue
            for license_file in crate.rglob('*'):
                if license_file.is_file() and any(word in license_file.name.lower() for word in ['license','copying','copyright']) and license_file.stat().st_size < 300_000:
                    archive.write(license_file, f'{prefix}/licenses/rust-dependencies/{crate.name}/{license_file.relative_to(crate)}')
    archive.writestr(f'{prefix}/START-HERE.txt', r'''OpenAIRecorder 0.4 — Gemini · Native Windows 10/11 x64

Extract the folder and double-click openairecorder.exe.
Keep libunwind.dll beside the executable. No setup scripts are required.
Whisper and subscription CLI agents are no longer used.

Open Settings and paste your Gemini API key. Refresh the available models,
choose an audio-capable model, and use Test audio & notes. The test makes
an API request using one second of generated silence. If the audio model
cannot return structured notes, the app asks for a separate notes model.
Save your settings. Gemini API quota, pricing, and model availability apply.

Start recording after informing participants that audio goes to Gemini.
Each completed minute is uploaded automatically; transcript, Markdown notes,
and actions update after the response. This is minute-batch processing,
not word-by-word streaming. Language is detected automatically.

The discreet, movable topmost strip has a timer, pause/resume, stop, and green
speech-activity bars. Local WebRTC VAD drives the bars without waiting for AI.
Pause omits new audio and timer time; queued parts may finish. Stop also sends
the final partial minute. Pending audio and successful responses are kept for
retry/recovery. Use Retry pending audio after key/model/quota/network errors.

Settings > Your data > Move data folder opens a folder picker. Select an
empty folder and confirm. Finish recording/processing and save edits first.
All workspace files are copied and verified before the location switches;
the old files are then removed. A small workspace-location.json stays in
the original app configuration area so future launches use the new folder.

Your key is encrypted for your Windows account and stored separately from
recordings and Markdown. Audio/notes themselves are not encrypted. Recordings
normally live under %LOCALAPPDATA%\OpenAIRecorder\recordings; older portable
data is reused. The app never writes the API key into exports or provider URLs.

Global Actions lets you add personal tasks and edit copied meeting tasks,
set due dates with a calendar, pin, search, and sort by due or added date.
Overdue/today/tomorrow tasks rise above pins; completed tasks stay last.
Deadline indicators use your computer's local calendar date. Global edits
never change the source meeting task. Older tasks without an added date
remain editable; relative deadlines need an exact date for reminders.

Existing/imported mono 16 kHz PCM16 WAV can be processed in minute batches.
No ffmpeg or local speech-model installation is required.

Built and tested on Linux with a simulated Gemini API. Actual Google accuracy,
quota/authentication, Windows capture, DPAPI and widget behavior need local
Windows validation with your own API key. See README.md / VALIDATION.md.
''')

digest = hashlib.sha256(output.read_bytes()).hexdigest()
output.with_suffix('.zip.sha256').write_text(f'{digest}  {output.name}\n')
print(f'Packaged {output.name} ({output.stat().st_size:,} bytes), SHA-256 {digest}')
