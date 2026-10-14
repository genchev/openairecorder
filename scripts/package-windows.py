"""Package a cross-built Windows app and verified third-party runtimes."""
import hashlib
from pathlib import Path
import zipfile

repo = Path(__file__).resolve().parent.parent
tools = Path('/workspace/.tools')
compiler = tools / 'llvm-mingw-20261006-ucrt-ubuntu-22.04-x86_64'
whisper = tools / 'whisper-windows-portable/bin/whisper-cli.exe'
assert hashlib.sha256((tools / 'llvm-mingw.tar.xz').read_bytes()).hexdigest() == '5f9c6ed95b2d4bdb2869a488c5fd5857fbdabcf288a0aa3eb1da43f6a08d8ab4'
output = repo / 'dist' / 'OpenAIRecorder-Native-Windows-x64.zip'
output.parent.mkdir(exist_ok=True)
prefix = 'OpenAIRecorder'
with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED, strict_timestamps=False) as archive:
    archive.write(repo / 'target/x86_64-pc-windows-gnullvm/release/openairecorder.exe', f'{prefix}/openairecorder.exe')
    archive.write(compiler / 'x86_64-w64-mingw32/bin/libunwind.dll', f'{prefix}/libunwind.dll')
    archive.write(tools / 'llvm-LICENSE.txt', f'{prefix}/licenses/LLVM.txt')
    archive.write(compiler / 'x86_64-w64-mingw32/share/mingw32/COPYING', f'{prefix}/licenses/mingw-w64.txt')
    archive.write(tools / 'whisper.cpp/LICENSE', f'{prefix}/licenses/whisper.cpp.txt')
    for name in ['README.md', 'VALIDATION.md', 'docs/screenshots/conversation-dark.png', 'docs/screenshots/conversation-light.png']:
        archive.write(repo / name, f'{prefix}/{name}')
    archive.write(whisper, f'{prefix}/.local-tools/whisper/Release/whisper-cli.exe')
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
    model = tools / 'whisper.cpp/models/ggml-tiny.bin'
    if model.is_file():
        assert hashlib.sha256(model.read_bytes()).hexdigest() == 'be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21'
        archive.write(model, f'{prefix}/.local-tools/ggml-tiny.bin')
    archive.writestr(f'{prefix}/START-HERE.txt', r'''OpenAIRecorder 0.3 — Native Windows 10/11 x64

Extract the entire folder, then double-click openairecorder.exe.
No setup scripts, PowerShell, Rust installation, or Visual C++ installer is
required for recording, WAV playback, and local speech recognition.
Keep libunwind.dll and the .local-tools folder beside the executable.

Whisper and its multilingual tiny model are included. Bulgarian is the default
speech language. Change it in Settings for English or automatic detection.
You can select a larger multilingual Whisper model in Settings for improved
recognition; tiny is fast but can make mistakes, especially in Bulgarian.

For AI notes, the app detects installed Codex or Claude Code agents. Install and
sign in through the provider's official setup process if needed; Settings links
to their guides. Existing subscription logins are reused. No API key is needed.
This app does not override company restrictions on installing software.

Record, Pause/Resume, then Stop & save. Open the conversation, create its
transcript, then choose an agent under Audio & AI. Generated or imported
Markdown is rendered in Overview. Check actions off or add open actions to the
global list. Import Markdown with task checkboxes (- [ ] / - [x]) using the GUI.

Native recordings need no ffmpeg. Other imported audio formats may need an
existing ffmpeg installation for transcription. Mono 16 kHz PCM16 WAV works
without it. The app does not ask you to run an installation script.

Data: %LOCALAPPDATA%\OpenAIRecorder\recordings (legacy portable data is reused).
The build was cross-compiled on Linux. Physical Windows audio and floating
widget behavior still require Windows testing. See README.md / VALIDATION.md.
''')

digest = hashlib.sha256(output.read_bytes()).hexdigest()
output.with_suffix('.zip.sha256').write_text(f'{digest}  {output.name}\n')
print(f'Packaged {output.name} ({output.stat().st_size:,} bytes), SHA-256 {digest}')
