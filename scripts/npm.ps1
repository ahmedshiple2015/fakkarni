$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$env:PATH = "$cargoBin;$env:PATH"
$localNpm = Join-Path $projectRoot '.tools\package\bin\npm-cli.js'
if (Test-Path -LiteralPath $localNpm) {
    & node $localNpm @args
} else {
    & npm.cmd @args
}
exit $LASTEXITCODE
