$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location -LiteralPath $root
$env:PATH = "$(Join-Path $env:USERPROFILE '.cargo\bin');$env:PATH"
$env:CARGO_TARGET_DIR = Join-Path $root 'target'
cargo build --release --locked -p fakkarni-native-host
if ($LASTEXITCODE -ne 0) { throw 'Native bridge build failed.' }
node extension/build.mjs
if ($LASTEXITCODE -ne 0) { throw 'Extension build failed.' }
$assets = Join-Path $root 'target/release-assets/browser-extension'
New-Item -ItemType Directory -Path $assets -Force | Out-Null
foreach ($file in @('manifest.json', 'popup.html', 'popup.css', 'theme.css')) {
    Copy-Item -LiteralPath (Join-Path "$root/extension" $file) -Destination $assets -Force
}
foreach ($directory in @('dist', 'icons')) {
    $target = Join-Path $assets $directory
    New-Item -ItemType Directory -Path $target -Force | Out-Null
    Get-ChildItem -LiteralPath (Join-Path "$root/extension" $directory) -File |
        ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $target -Force }
}
node node_modules/@tauri-apps/cli/tauri.js build --config src-tauri/tauri.release.conf.json --bundles nsis
if ($LASTEXITCODE -ne 0) { throw 'Windows installer build failed.' }
$version = (Get-Content -Raw -LiteralPath "$root/src-tauri/tauri.release.conf.json" | ConvertFrom-Json).version
$bundle = Join-Path $root 'target/release/bundle/nsis'
$installer = @(Get-ChildItem -LiteralPath $bundle -File -Filter "*_${version}_*-setup.exe")
if ($installer.Count -ne 1) { throw 'Could not identify one Windows installer.' }
$output = Join-Path $root 'artifacts/release'
New-Item -ItemType Directory -Path $output -Force | Out-Null
$setup = Join-Path $output $installer[0].Name
Copy-Item -LiteralPath $installer[0].FullName -Destination $setup -Force
$archive = Join-Path $output "Fakkarni-$version-extension.zip"
Compress-Archive -Path "$assets/*" -DestinationPath $archive -Force
Get-FileHash -Algorithm SHA256 -LiteralPath $setup, $archive | ForEach-Object {
    "$($_.Hash.ToLowerInvariant())  $(Split-Path $_.Path -Leaf)"
} | Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS.txt') -Encoding ascii
