$ErrorActionPreference = 'Stop'
$Repo = Split-Path $PSScriptRoot -Parent
Set-Location $Repo
$Tools = Join-Path $Repo '.local-tools'
$Binary = Get-ChildItem (Join-Path $Tools 'whisper') -Filter whisper-cli.exe -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
if ($Binary) { $env:WHISPER_BIN = $Binary.FullName }
if (-not $env:WHISPER_MODEL) { $env:WHISPER_MODEL = Join-Path $Tools 'ggml-tiny.bin' }
if (Test-Path '.\openairecorder.exe') { & '.\openairecorder.exe' }
elseif (Test-Path '.\target\release\openairecorder.exe') { & '.\target\release\openairecorder.exe' }
else { cargo run --locked }
if ($LASTEXITCODE -ne 0) { throw 'OpenAIRecorder exited with an error.' }
