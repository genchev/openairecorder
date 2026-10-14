"""Package a cross-built Windows app and verified third-party runtimes."""
import hashlib
from pathlib import Path
import zipfile

repo = Path(__file__).resolve().parent.parent
tools = Path('/workspace/.tools')
compiler = tools / 'llvm-mingw-20261006-ucrt-ubuntu-22.04-x86_64'
whisper = tools / 'whisper-windows-v1.8.7.zip'
assert hashlib.sha256(whisper.read_bytes()).hexdigest() == 'd9627486e1c34a03745880485593473e047294260ce9a3cb0aa8deaf15b99af6'
assert hashlib.sha256((tools / 'llvm-mingw.tar.xz').read_bytes()).hexdigest() == '5f9c6ed95b2d4bdb2869a488c5fd5857fbdabcf288a0aa3eb1da43f6a08d8ab4'
output = repo / 'dist' / 'OpenAIRecorder-Windows-x64.zip'
output.parent.mkdir(exist_ok=True)
prefix = 'OpenAIRecorder'
with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
    archive.write(repo / 'target/x86_64-pc-windows-gnullvm/release/openairecorder.exe', f'{prefix}/openairecorder.exe')
    archive.write(compiler / 'x86_64-w64-mingw32/bin/libunwind.dll', f'{prefix}/libunwind.dll')
    archive.write(tools / 'llvm-LICENSE.txt', f'{prefix}/licenses/LLVM.txt')
    archive.write(compiler / 'x86_64-w64-mingw32/share/mingw32/COPYING', f'{prefix}/licenses/mingw-w64.txt')
    archive.write(tools / 'whisper.cpp/LICENSE', f'{prefix}/licenses/whisper.cpp.txt')
    for name in ['README.md', 'scripts/setup-windows.ps1', 'scripts/start-windows.ps1']:
        archive.write(repo / name, f'{prefix}/{name}')
    with zipfile.ZipFile(whisper) as binaries:
        for name in ['whisper-cli.exe', 'whisper.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu.dll']:
            archive.writestr(f'{prefix}/.local-tools/whisper/Release/{name}', binaries.read(f'Release/{name}'))
    model = tools / 'whisper.cpp/models/ggml-tiny.bin'
    if model.is_file():
        assert hashlib.sha256(model.read_bytes()).hexdigest() == 'be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21'
        archive.write(model, f'{prefix}/.local-tools/ggml-tiny.bin')
    archive.writestr(f'{prefix}/START-HERE.txt', '''OpenAIRecorder — Windows 10/11 x64 prototype

1. Extract this whole folder to a writable location on your computer.
2. In PowerShell install dependencies:
   winget install --id Gyan.FFmpeg --exact
   winget install --id Microsoft.VCRedist.2015+.x64 --exact
3. Open a NEW PowerShell in the extracted OpenAIRecorder folder:
   .\\scripts\\setup-windows.ps1
   (Downloads and verifies the local speech model if not bundled.)
4. Install and sign in to Codex or Claude Code. The GUI also shows instructions:
   npm install -g @openai/codex
   codex login
   OR: winget install --id Anthropic.ClaudeCode --exact
       claude auth login
5. Run .\\scripts\\start-windows.ps1 (no Rust installation required).
   The GUI opens in Edge/Chrome. Keep the server and GUI open during recording.

Notes and recordings stay in the recordings folder. The chosen CLI receives
transcript text only when you confirm generating notes. Subscription limits apply.

This executable was cross-built on Linux. Native widget behavior and Windows
system-audio capture still need validation on a Windows computer.
See README.md for capture instructions, settings, and current limitations.
''')
digest = hashlib.sha256(output.read_bytes()).hexdigest()
output.with_suffix('.zip.sha256').write_text(f'{digest}  {output.name}\n')
print(f'Packaged {output.name} ({output.stat().st_size:,} bytes), SHA-256 {digest}')
