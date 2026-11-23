# Build an UNTESTED portable Windows x86_64 bundle.
# Pair the printer in Settings > Bluetooth & devices before running.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$Version = if ($env:VERSION) { $env:VERSION } else { "0.4.0" }
$Version = $Version -replace '^v', ''

cargo build --release --locked -p liene-cli -p liene-gui
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Name = "Liene-Desktop-App-$Version-windows-x86_64"
$Stage = "dist/$Name"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage -Force | Out-Null

Copy-Item "target/release/liene-cli.exe", "target/release/liene-desktop-app.exe" $Stage
Copy-Item "README.md", "LICENSE" $Stage

@"
UNTESTED WINDOWS BUILD

This Windows build is provided for testing. It is compile-checked in CI but
has not been verified on real N200 Pro hardware.

1. Pair the printer in Settings > Bluetooth & devices > Add device.
2. Keep liene-cli.exe and liene-desktop-app.exe in the same folder.
3. Run liene-desktop-app.exe.

If it works, please open an issue with your Windows version, printer
model/firmware, and a `liene-cli.exe --debug status` log. Help wanted:
https://github.com/bydavy/liene-printer-desktop-app/issues
"@ | Set-Content "$Stage/README-UNTESTED.txt" -Encoding utf8

$Zip = "dist/$Name.zip"
if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path "$Stage/*" -DestinationPath $Zip -Force

$Hash = (Get-FileHash $Zip -Algorithm SHA256).Hash.ToLower()
"$Hash  $(Split-Path $Zip -Leaf)" | Set-Content "$Zip.sha256" -Encoding ascii
Write-Host "Built: $Zip (untested)"
