$ErrorActionPreference = 'Stop'
$registryPath = 'HKCU:\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\org.fakkarni.bridge'
if (Test-Path -LiteralPath $registryPath) {
    # Removes only this app's host registration. No vault or credential files are touched.
    Remove-Item -LiteralPath $registryPath
}
Write-Output 'Fakkarni native host registration removed for this user. Disable/remove the extension separately in Brave.'
