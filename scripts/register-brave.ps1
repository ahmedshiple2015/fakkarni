param(
    [ValidateSet('Brave', 'Edge')][string]$Browser = 'Brave',
    [string]$BinaryPath = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\debug\fakkarni-native-host.exe'),
    [string]$DesktopPath = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\debug\fakkarni.exe')
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$identity = Get-Content -Raw (Join-Path $projectRoot 'extension\identity.json') | ConvertFrom-Json
if ($identity.extensionId -notmatch '^[a-p]{32}$' -or $identity.nativeHost -ne 'org.fakkarni.bridge') {
    throw 'Invalid browser extension identity.'
}
$binary = (Resolve-Path -LiteralPath $BinaryPath).Path
if ([IO.Path]::GetExtension($binary) -ne '.exe') { throw 'Expected the compiled native host executable.' }
$destination = Join-Path $env:LOCALAPPDATA 'org.fakkarni.desktop\native-messaging'
New-Item -ItemType Directory -Force -Path $destination | Out-Null
$installedBinary = Join-Path $destination 'fakkarni-native-host.exe'
$desktopBinary = (Resolve-Path -LiteralPath $DesktopPath).Path
if ([IO.Path]::GetExtension($desktopBinary) -ne '.exe') { throw 'Expected the desktop application executable.' }
Copy-Item -LiteralPath $binary -Destination $installedBinary -Force
$launchConfig = @{ desktopExeFullPath = $desktopBinary } | ConvertTo-Json
[IO.File]::WriteAllText((Join-Path $destination 'desktop-path.json'), $launchConfig, [Text.UTF8Encoding]::new($false))
$manifestPath = Join-Path $destination 'org.fakkarni.bridge.json'
$manifest = @{
    name = $identity.nativeHost
    description = 'Fakkarni local account suggestions'
    path = $installedBinary
    type = 'stdio'
    allowed_origins = @("chrome-extension://$($identity.extensionId)/")
}
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
# Brave builds can inherit Chromium's Chrome-compatible native-host lookup.
# The manifest still permits only Fakkarni's extension.
$registryPaths = @(
    'HKCU:\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\org.fakkarni.bridge',
    'HKCU:\Software\Google\Chrome\NativeMessagingHosts\org.fakkarni.bridge'
)
if ($Browser -eq 'Edge') {
    $registryPaths = @('HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\org.fakkarni.bridge')
}
foreach ($registryPath in $registryPaths) {
    New-Item -Path $registryPath -Force | Out-Null
    Set-Item -LiteralPath $registryPath -Value $manifestPath
    if ((Get-Item -LiteralPath $registryPath).GetValue('') -ne $manifestPath) {
        throw 'Could not verify Fakkarni native host registration.'
    }
}
Write-Output "Fakkarni native host registered for $Browser for this Windows user only."
Write-Output 'Load the unpacked extension directory in your browser, then open and unlock Fakkarni.'
