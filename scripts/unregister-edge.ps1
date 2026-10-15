$ErrorActionPreference = 'Stop'
$registryPath = 'HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\org.fakkarni.bridge'
if (Test-Path -LiteralPath $registryPath) {
    Remove-Item -LiteralPath $registryPath
}
Write-Output 'Fakkarni native host registration removed for Edge for this user. Disable/remove the extension separately in Edge.'
