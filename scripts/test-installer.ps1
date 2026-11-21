$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Use a disposable Windows CI runner.' }
$root = Split-Path $PSScriptRoot -Parent
$setup = @(Get-ChildItem -LiteralPath "$root/artifacts/release" -Filter '*-setup.exe')
if ($setup.Count -ne 1) { throw 'Build exactly one installer before testing.' }
$data = Join-Path $env:LOCALAPPDATA 'org.fakkarni.desktop'
$install = Join-Path $env:RUNNER_TEMP 'Fakkarni Installer Smoke Test'
if ((Test-Path -LiteralPath $data) -or (Test-Path -LiteralPath $install)) {
    throw 'The smoke test requires a clean user profile.'
}
$manifest = Join-Path $data 'native-messaging/org.fakkarni.bridge.json'
$run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$desktop = Join-Path $install 'fakkarni.exe'
$vendorPaths = @(
    'BraveSoftware\Brave-Browser',
    'Google\Chrome',
    'Microsoft\Edge'
)
function Assert-Bridge([bool]$present) {
    foreach ($view in @('Registry32', 'Registry64')) {
        $hive = [Microsoft.Win32.RegistryKey]::OpenBaseKey('CurrentUser', $view)
        try {
            foreach ($vendor in $vendorPaths) {
                $key = $hive.OpenSubKey("Software\$vendor\NativeMessagingHosts\org.fakkarni.bridge")
                try {
                    $value = if ($null -ne $key) { $key.GetValue('') }
                    if ($present -and $value -ne [IO.Path]::GetFullPath($manifest)) {
                        throw "Missing or unexpected registration for $vendor ($view)."
                    }
                    if (!$present -and $null -ne $value) {
                        throw "Registration was left behind for $vendor ($view)."
                    }
                } finally {
                    if ($null -ne $key) { $key.Dispose() }
                }
            }
        } finally {
            $hive.Dispose()
        }
    }
}
function Invoke-Installer([string]$path, [string]$arguments) {
    $process = Start-Process -FilePath $path -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer returned $($process.ExitCode): $path" }
}
function Get-Startup {
    Get-ItemPropertyValue -LiteralPath $run -Name Fakkarni -ErrorAction SilentlyContinue
}

Assert-Bridge $false
if (Get-Startup) { throw 'The smoke test requires no existing Fakkarni startup entry.' }
Invoke-Installer $setup[0].FullName "/S /D=$install"
Assert-Bridge $true
if (Get-Startup) { throw 'Clean installation enabled startup without an opt-in.' }
$native = Get-Content -Raw -LiteralPath $manifest | ConvertFrom-Json
$extension = Get-Content -Raw -LiteralPath "$install/browser-extension/manifest.json" | ConvertFrom-Json
if (!(Test-Path -LiteralPath $desktop) -or !(Test-Path -LiteralPath $native.path) -or
    !(Test-Path -LiteralPath "$install/browser-extension/$($extension.background.service_worker)")) {
    throw 'The installer is missing its desktop, native host, or extension worker.'
}
$vault = Join-Path $data 'vault.kdbx'
Set-Content -LiteralPath $vault -Value 'synthetic installer preservation fixture'
$hash = (Get-FileHash -LiteralPath $vault).Hash
if (!(Test-Path -LiteralPath $run)) { New-Item -Path $run | Out-Null }
Set-ItemProperty -LiteralPath $run -Name Fakkarni -Value '"C:\old\fakkarni.exe" --autostart'
Invoke-Installer $setup[0].FullName "/S /UPDATE /D=$install"
Assert-Bridge $true
if ((Get-Startup) -ne ('"{0}" --autostart' -f $desktop)) { throw 'Startup opt-in was not migrated.' }
if ((Get-FileHash -LiteralPath $vault).Hash -ne $hash) { throw 'Reinstall changed the vault fixture.' }
Invoke-Installer "$install/uninstall.exe" '/S'
Assert-Bridge $false
if ((Get-Startup) -or (Test-Path -LiteralPath $manifest) -or (Test-Path -LiteralPath $desktop)) {
    throw 'Uninstall left installed files or startup registration behind.'
}
if ((Get-FileHash -LiteralPath $vault).Hash -ne $hash) { throw 'Uninstall changed the vault fixture.' }
Write-Output 'Passed install, registration, reinstall, startup migration, and data-preserving uninstall.'
