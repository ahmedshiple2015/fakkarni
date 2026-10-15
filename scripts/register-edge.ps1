param(
    [string]$BinaryPath = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\debug\fakkarni-native-host.exe')
)
$ErrorActionPreference = 'Stop'
# Reuse the shared binary and manifest installation without changing Brave's registration.
& (Join-Path $PSScriptRoot 'register-brave.ps1') -Browser Edge -BinaryPath $BinaryPath
