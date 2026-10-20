param([switch]$Probe)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$identity = Get-Content -Raw (Join-Path $root 'extension\identity.json') | ConvertFrom-Json
$destination = Join-Path $env:LOCALAPPDATA 'org.fakkarni.desktop\native-messaging'
$expectedManifest = Join-Path $destination 'org.fakkarni.bridge.json'
$result = [ordered]@{ extensionId = $identity.extensionId; manifestPath = $expectedManifest; registry = @(); manifestExists = (Test-Path -LiteralPath $expectedManifest); manifestValid = $false; hostExists = $false; hostMatchesBuild = $null; desktopExists = $false; probe = 'Not requested' }
foreach ($browser in @('BraveSoftware\Brave-Browser', 'Google\Chrome', 'Microsoft\Edge')) {
    foreach ($view in @([Microsoft.Win32.RegistryView]::Registry32, [Microsoft.Win32.RegistryView]::Registry64)) {
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser, $view)
        $key = $null
        try {
            $key = $base.OpenSubKey("Software\$browser\NativeMessagingHosts\org.fakkarni.bridge")
            $value = if ($null -ne $key) { $key.GetValue('') } else { $null }
            $result.registry += [ordered]@{ browser = $browser; view = "$view"; registered = ($null -ne $value); pointsToExpectedManifest = ($value -ceq $expectedManifest) }
        } finally { if ($null -ne $key) { $key.Dispose() }; $base.Dispose() }
    }
}
$manifest = $null
if ($result.manifestExists) {
    try {
        if ((Get-Item -LiteralPath $expectedManifest).Length -gt 16384) { throw 'Manifest exceeds limit.' }
        $manifest = [IO.File]::ReadAllText($expectedManifest) | ConvertFrom-Json
        $allowed = @($manifest.allowed_origins)
        $result.manifestValid = ($manifest.name -ceq 'org.fakkarni.bridge' -and $manifest.type -ceq 'stdio' -and $allowed.Count -eq 1 -and $allowed[0] -ceq "chrome-extension://$($identity.extensionId)/" -and [IO.Path]::IsPathRooted($manifest.path))
        $result.hostExists = Test-Path -LiteralPath $manifest.path
        $build = Join-Path $root 'target\debug\fakkarni-native-host.exe'
        if ($result.hostExists -and (Test-Path -LiteralPath $build)) { $result.hostMatchesBuild = ((Get-FileHash -LiteralPath $manifest.path).Hash -eq (Get-FileHash -LiteralPath $build).Hash) }
    } catch { $result.manifestValid = $false }
}
try {
    $configPath = Join-Path $destination 'desktop-path.json'
    if ((Get-Item -LiteralPath $configPath).Length -gt 8192) { throw 'Config exceeds limit.' }
    $config = [IO.File]::ReadAllText($configPath) | ConvertFrom-Json
    $result.desktopExists = [IO.Path]::IsPathRooted($config.desktopExeFullPath) -and (Test-Path -LiteralPath $config.desktopExeFullPath)
} catch { $result.desktopExists = $false }
if ($Probe -and $result.manifestValid -and $result.hostExists) {
    if (-not ('FakkarniBridgeDiagnostics' -as [type])) {
        Add-Type -TypeDefinition @'
using System.IO;
using System.Threading.Tasks;
public static class FakkarniBridgeDiagnostics {
    public static async Task<byte[]> ReadBoundedAsync(Stream input) {
        const int limit = 32773;
        using (var output = new MemoryStream()) {
            var chunk = new byte[4096];
            while (output.Length < limit) {
                int remaining = limit - (int)output.Length;
                int count = await input.ReadAsync(chunk, 0, remaining < chunk.Length ? remaining : chunk.Length);
                if (count == 0) break;
                output.Write(chunk, 0, count);
            }
            return output.ToArray();
        }
    }
}
'@
    }
    # Synthetic origin only. Print status/count, never response bodies, credentials,
    # saved account metadata, URLs, browser history, or diagnostic browser logs.
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $manifest.path
    $start.Arguments = "chrome-extension://$($identity.extensionId)/"
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        [void]$process.Start()
        $payload = [Text.Encoding]::UTF8.GetBytes('{"action":"match","version":1,"origin":"https://example.test"}')
        $header = [BitConverter]::GetBytes([uint32]$payload.Length)
        $process.StandardInput.BaseStream.Write($header, 0, $header.Length)
        $process.StandardInput.BaseStream.Write($payload, 0, $payload.Length)
        $process.StandardInput.Close()
        $copy = [FakkarniBridgeDiagnostics]::ReadBoundedAsync($process.StandardOutput.BaseStream)
        if (-not $process.WaitForExit(6000)) { $process.Kill(); $result.probe = 'Host timeout' }
        elseif (-not $copy.Wait(1000)) { $result.probe = 'Host exited but output pipe remained open' }
        else {
            $bytes = $copy.Result
            if ($bytes.Length -lt 4 -or $bytes.Length -gt 32772 -or [BitConverter]::ToUInt32($bytes, 0) -ne $bytes.Length - 4) { $result.probe = 'Invalid framing' }
            else {
                $reply = [Text.Encoding]::UTF8.GetString($bytes, 4, $bytes.Length - 4) | ConvertFrom-Json
                $validStatuses = @('ok', 'locked', 'unavailable', 'error')
                $result.probe = if ($validStatuses -contains $reply.status) { "Protocol status: $($reply.status); match count: $(@($reply.matches).Count)" } else { 'Invalid response status' }
            }
        }
    } catch { $result.probe = 'Probe failed' }
    finally { $process.Dispose() }
}
$result | ConvertTo-Json -Depth 5
