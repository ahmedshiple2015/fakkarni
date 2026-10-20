function Set-FakkarniNativeHostRegistration {
    param([string[]]$RegistryPaths, [string]$ManifestPath)
    foreach ($view in @([Microsoft.Win32.RegistryView]::Registry32, [Microsoft.Win32.RegistryView]::Registry64)) {
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, $view)
        try {
            foreach ($registryPath in $RegistryPaths) {
                if (-not $registryPath.StartsWith('HKCU:\Software\', [StringComparison]::OrdinalIgnoreCase)) {
                    throw 'Native host registration must stay under this user''s Software registry key.'
                }
                # CreateSubKey creates missing ancestors too. New-Item without
                # -Force failed on clean desktop installs with no parent key.
                $key = $base.CreateSubKey($registryPath.Substring(6))
                try {
                    if ($key.GetValue('') -cne $ManifestPath -or $key.GetValueKind('') -ne [Microsoft.Win32.RegistryValueKind]::String) {
                        $key.SetValue('', $ManifestPath, [Microsoft.Win32.RegistryValueKind]::String)
                    }
                    if ($key.GetValue('') -cne $ManifestPath) { throw 'Could not verify Fakkarni native host registration.' }
                } finally { $key.Dispose() }
            }
        } finally { $base.Dispose() }
    }
}
