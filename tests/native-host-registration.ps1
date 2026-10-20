$ErrorActionPreference = 'Stop'
. (Join-Path (Split-Path $PSScriptRoot -Parent) 'scripts\native-host-registration.ps1')
$testRoot = 'Software\FakkarniRegistrationTests\' + [Guid]::NewGuid().ToString('N')
$relative = $testRoot + '\missing\parents\NativeMessagingHosts\org.fakkarni.synthetic'
$path = 'HKCU:\' + $relative
$manifest = 'C:\Synthetic Fixture\bridge.json'
try {
    Set-FakkarniNativeHostRegistration -RegistryPaths @($path) -ManifestPath $manifest
    Set-FakkarniNativeHostRegistration -RegistryPaths @($path) -ManifestPath $manifest
    foreach ($view in @([Microsoft.Win32.RegistryView]::Registry32, [Microsoft.Win32.RegistryView]::Registry64)) {
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, $view)
        $key = $base.OpenSubKey($relative)
        try {
            if (-not $key -or $key.GetValue('') -cne $manifest -or $key.GetValueKind('') -ne [Microsoft.Win32.RegistryValueKind]::String) { throw 'Missing-parent registration failed.' }
        } finally { if ($key) { $key.Dispose() }; $base.Dispose() }
    }
    $refused = $false
    try { Set-FakkarniNativeHostRegistration -RegistryPaths @('HKLM:\Software\FakkarniSynthetic') -ManifestPath $manifest }
    catch { $refused = $true }
    if (-not $refused) { throw 'Non-user registration was not rejected.' }
    Write-Output 'PASS: missing ancestors, both registry views, repeat installation, REG_SZ, and user-only scope'
} finally {
    if ($testRoot -notmatch '^Software\\FakkarniRegistrationTests\\[a-f0-9]{32}$') { throw 'Unexpected test registry path.' }
    foreach ($view in @([Microsoft.Win32.RegistryView]::Registry32, [Microsoft.Win32.RegistryView]::Registry64)) {
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, $view)
        try { $base.DeleteSubKeyTree($testRoot, $false) } finally { $base.Dispose() }
    }
}
