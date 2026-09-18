# Lingua portable zip (Windows)

param(
  [string]$Version = "0.1.0"
)

$ErrorActionPreference = "Stop"
$exe = Join-Path $PSScriptRoot "..\src-tauri\target\release\lingua.exe"
if (-not (Test-Path $exe)) {
  throw "Build first: npm run tauri build"
}
$out = Join-Path $PSScriptRoot "..\lingua-$Version-windows-x64.zip"
if (Test-Path $out) { Remove-Item $out }
Compress-Archive -Path (Resolve-Path $exe) -DestinationPath $out
Write-Output $out
