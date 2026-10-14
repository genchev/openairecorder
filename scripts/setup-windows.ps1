# Run from PowerShell: .\scripts\setup-windows.ps1
# Installs local transcription only. Install/sign in to your chosen agent separately.
$ErrorActionPreference = 'Stop'
$Repo = Split-Path $PSScriptRoot -Parent
$Tools = Join-Path $Repo '.local-tools'
New-Item -ItemType Directory -Force $Tools | Out-Null
if (-not (Get-Command ffmpeg -ErrorAction SilentlyContinue)) {
    Write-Host 'Install ffmpeg with: winget install --id Gyan.FFmpeg --exact'
    throw 'ffmpeg not found. Install it, then open a new PowerShell and rerun this script.'
}
Write-Host 'Whisper also needs the Microsoft Visual C++ runtime. If missing: winget install --id Microsoft.VCRedist.2015+.x64 --exact'
$Whisper = Join-Path $Tools 'whisper'
$Binary = Get-ChildItem $Whisper -Filter whisper-cli.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $Binary) {
    $Archive = Join-Path $Tools 'whisper-v1.8.7-x64.zip'
    Invoke-WebRequest 'https://github.com/ggml-org/whisper.cpp/releases/download/v1.8.7/whisper-bin-x64.zip' -OutFile $Archive
    if ((Get-FileHash $Archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne 'd9627486e1c34a03745880485593473e047294260ce9a3cb0aa8deaf15b99af6') { Remove-Item $Archive; throw 'Whisper archive checksum mismatch.' }
    Expand-Archive $Archive -DestinationPath $Whisper -Force
    $Binary = Get-ChildItem $Whisper -Filter whisper-cli.exe -Recurse | Select-Object -First 1
    if (-not $Binary) { throw 'The Whisper archive contains no whisper-cli.exe.' }
}
$Model = Join-Path $Tools 'ggml-tiny.bin'
if (-not (Test-Path $Model)) {
    $Temporary = "$Model.download"
    Invoke-WebRequest 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin?download=true' -OutFile $Temporary
    if ((Get-FileHash $Temporary -Algorithm SHA256).Hash.ToLowerInvariant() -ne 'be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21') { Remove-Item $Temporary; throw 'Speech model checksum mismatch.' }
    Move-Item $Temporary $Model -Force
}
if ((Get-FileHash $Model -Algorithm SHA256).Hash.ToLowerInvariant() -ne 'be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21') { throw 'Existing tiny model checksum mismatch.' }
$env:WHISPER_BIN = $Binary.FullName
$env:WHISPER_MODEL = $Model
Write-Host 'Local transcription installed. Start with .\scripts\start-windows.ps1'
