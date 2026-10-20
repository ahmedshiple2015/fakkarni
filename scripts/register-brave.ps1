param(
    [ValidateSet('Brave', 'Edge')][string]$Browser = 'Brave',
    [string]$BinaryPath = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\debug\fakkarni-native-host.exe'),
    [string]$DesktopPath = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\debug\fakkarni.exe')
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'native-host-registration.ps1')
$projectRoot = Split-Path $PSScriptRoot -Parent
$identity = Get-Content -Raw (Join-Path $projectRoot 'extension\identity.json') | ConvertFrom-Json
if ($identity.extensionId -notmatch '^[a-p]{32}$' -or $identity.nativeHost -ne 'org.fakkarni.bridge') {
    throw 'Invalid browser extension identity.'
}
$binary = (Resolve-Path -LiteralPath $BinaryPath).Path
$desktopBinary = (Resolve-Path -LiteralPath $DesktopPath).Path
if ([IO.Path]::GetExtension($binary) -ne '.exe' -or [IO.Path]::GetExtension($desktopBinary) -ne '.exe') {
    throw 'Expected the compiled native host and desktop executables.'
}
$destination = Join-Path $env:LOCALAPPDATA 'org.fakkarni.desktop\native-messaging'
New-Item -ItemType Directory -Force -Path $destination | Out-Null

# Serialize Brave/Edge registrations: both deliberately share the same installed host.
$accountSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$registrationGate = [Threading.Mutex]::new($false, "Local\FakkarniBrowserRegistration-$accountSid")
$gateHeld = $false
function Install-AtomicFile {
    param([string]$Target, [string]$Source, [byte[]]$Bytes)
    if (-not $Source -and $Bytes.Length -gt 16384) { throw 'Browser bridge metadata exceeds its size limit.' }
    if (Test-Path -LiteralPath $Target) {
        if ($Source) {
            if ((Get-FileHash -LiteralPath $Source -Algorithm SHA256).Hash -eq (Get-FileHash -LiteralPath $Target -Algorithm SHA256).Hash) { return }
        } else {
            if ((Get-Item -LiteralPath $Target).Length -le 16384) {
                $existing = [IO.File]::ReadAllBytes($Target)
                if ([Convert]::ToBase64String($existing) -ceq [Convert]::ToBase64String($Bytes)) { return }
            }
        }
    }
    $temporary = Join-Path (Split-Path $Target -Parent) ([IO.Path]::GetRandomFileName())
    $backup = Join-Path (Split-Path $Target -Parent) ([IO.Path]::GetRandomFileName())
    try {
        if ($Source) { [IO.File]::Copy($Source, $temporary, $false) }
        else { [IO.File]::WriteAllBytes($temporary, $Bytes) }
        # Flush the completed temporary file before publishing its name.
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if (Test-Path -LiteralPath $Target) { [IO.File]::Replace($temporary, $Target, $backup) }
        else { [IO.File]::Move($temporary, $Target) }
    } catch {
        throw "Could not atomically update $([IO.Path]::GetFileName($Target)). The previous installed file was preserved. If the browser bridge is running, close the browser and retry. $($_.Exception.Message)"
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
        if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Force }
    }
}
try {
    try { $gateHeld = $registrationGate.WaitOne(10000) }
    catch [Threading.AbandonedMutexException] { $gateHeld = $true }
    if (-not $gateHeld) { throw 'Another Fakkarni bridge registration is in progress. Retry shortly.' }
    $installedBinary = Join-Path $destination 'fakkarni-native-host.exe'
    Install-AtomicFile -Target $installedBinary -Source $binary
    $encoding = [Text.UTF8Encoding]::new($false)
    $launchConfig = [ordered]@{ desktopExeFullPath = $desktopBinary } | ConvertTo-Json
    Install-AtomicFile -Target (Join-Path $destination 'desktop-path.json') -Bytes $encoding.GetBytes($launchConfig)
    $manifestPath = Join-Path $destination 'org.fakkarni.bridge.json'
    $manifest = [ordered]@{
        name = $identity.nativeHost
        description = 'Fakkarni local account suggestions and autofill'
        path = $installedBinary
        type = 'stdio'
        allowed_origins = @("chrome-extension://$($identity.extensionId)/")
    } | ConvertTo-Json -Depth 4
    Install-AtomicFile -Target $manifestPath -Bytes $encoding.GetBytes($manifest)
    # Brave builds can inherit Chromium's Chrome-compatible native-host lookup.
    $registryPaths = @(
        'HKCU:\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\org.fakkarni.bridge',
        'HKCU:\Software\Google\Chrome\NativeMessagingHosts\org.fakkarni.bridge'
    )
    if ($Browser -eq 'Edge') { $registryPaths = @('HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\org.fakkarni.bridge') }
    Set-FakkarniNativeHostRegistration -RegistryPaths $registryPaths -ManifestPath $manifestPath
} finally {
    if ($gateHeld) { $registrationGate.ReleaseMutex() }
    $registrationGate.Dispose()
}
Write-Output "Fakkarni native host registered for $Browser for this Windows user only. Unchanged files and registry values are preserved."
Write-Output 'Load the unpacked extension directory in your browser, then open and unlock Fakkarni.'
