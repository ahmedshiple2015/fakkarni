param(
    [string]$BinaryPath = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\debug\fakkarni-native-host.exe')
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
Copy-Item -LiteralPath $binary -Destination $installedBinary -Force
$manifestPath = Join-Path $destination 'org.fakkarni.bridge.json'
$manifest = @{
    name = $identity.nativeHost
    description = 'Fakkarni local account suggestions'
    path = $installedBinary
    type = 'stdio'
    allowed_origins = @("chrome-extension://$($identity.extensionId)/")
}
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
$registryPath = 'HKCU:\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\org.fakkarni.bridge'
New-Item -Path $registryPath -Force | Out-Null
Set-Item -LiteralPath $registryPath -Value $manifestPath
Write-Output 'Fakkarni native host registered for Brave for this Windows user only.'
Write-Output 'Load the unpacked extension directory in brave://extensions, then open and unlock Fakkarni.'
