# Attiva ambiente isolato DualBond: CARGO_HOME locale al progetto,
# nessuna dipendenza da ~/.cargo globale. npm usa cache locale via .npmrc
# (gia' attiva di default, non serve sorgere nulla per npm).
#
# Uso: dot-source questo script prima di lavorare nel progetto:
#   . .\activate.ps1
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$env:CARGO_HOME = Join-Path $root ".cargo-home"
$env:PATH = "$($env:CARGO_HOME)\bin;$env:PATH"
Write-Host "DualBond env attivo."
Write-Host "  CARGO_HOME = $env:CARGO_HOME"
Write-Host "  npm cache  = $root\.npm-cache (via .npmrc)"
