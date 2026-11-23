# Build an UNTESTED portable Windows x86_64 bundle.
# Pair the printer in Settings > Bluetooth & devices before running.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$Version = if ($env:VERSION) { $env:VERSION } else { "0.5.0" }
$Version = $Version -replace '^v', ''

cargo build --release --locked -p liene-cli -p liene-gui
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Name = "Open-Liene-$Version-windows-x86_64"
$Stage = "dist/$Name"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage -Force | Out-Null

Copy-Item "target/release/open-liene-cli.exe", "target/release/open-desktop.exe" $Stage
Copy-Item "README.md", "LICENSE" $Stage

@"
UNTESTED WINDOWS BUILD

This Windows build is provided for testing. It is compile-checked in CI but
has not been verified on real N200 Pro hardware.

1. Pair the printer in Settings > Bluetooth & devices > Add device.
2. Keep open-liene-cli.exe and open-desktop.exe in the same folder.
3. Run open-desktop.exe.

If it works, please open an issue with your Windows version, printer
model/firmware, and a `open-liene-cli.exe --debug status` log. Help wanted:
https://github.com/bydavy/open-liene-printer-desktop/issues
"@ | Set-Content "$Stage/README-UNTESTED.txt" -Encoding utf8

$Zip = "dist/$Name.zip"
if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path "$Stage/*" -DestinationPath $Zip -Force

$Hash = (Get-FileHash $Zip -Algorithm SHA256).Hash.ToLower()
"$Hash  $(Split-Path $Zip -Leaf)" | Set-Content "$Zip.sha256" -Encoding ascii
Write-Host "Built: $Zip (untested)"
