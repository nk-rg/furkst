# Builds a standalone furkst.exe (static C runtime, no console window) into dist\.
# Run from the repository root: .\scripts\build-windows.ps1
$ErrorActionPreference = 'Stop'
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"').Matches[0].Groups[1].Value
cargo build --release --locked -p furkst
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
New-Item -ItemType Directory -Force dist | Out-Null
$zip = "dist\Furkst-$version-windows-x64.zip"
Copy-Item target\release\furkst.exe dist\furkst.exe -Force
Compress-Archive -Path dist\furkst.exe -DestinationPath $zip -Force
Write-Output "Built dist\furkst.exe and $zip"
