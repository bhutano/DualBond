# DualBond - one-line installer bootstrap.
#
#   powershell -ep bypass -c "irm https://raw.githubusercontent.com/bhutano/DualBond/main/install.ps1|iex"
#
# What it does, nothing else:
#   1. asks the GitHub API for the latest DualBond release,
#   2. downloads the Windows installer and its published SHA-256,
#   3. refuses to continue if the checksum does not match,
#   4. launches the installer (Windows will ask for administrator permission).
#
# Read it before running it: it is short on purpose.

$ErrorActionPreference = 'Stop'
$repo = 'bhutano/DualBond'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

Write-Host 'DualBond: looking for the latest release...'
$release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" `
    -Headers @{ 'User-Agent' = 'DualBond-installer' }

$exe = $release.assets | Where-Object { $_.name -like 'DualBond_*_x64-setup.exe' } | Select-Object -First 1
if (-not $exe) { throw 'No installer found in the latest release.' }
$sum = $release.assets | Where-Object { $_.name -eq "$($exe.name).sha256" } | Select-Object -First 1
if (-not $sum) { throw 'No published SHA-256 for the installer: refusing to continue.' }

$dir = Join-Path $env:TEMP ('dualbond-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dir | Out-Null
try {
    $path = Join-Path $dir $exe.name
    Write-Host "Downloading $($exe.name)..."
    Invoke-WebRequest -Uri $exe.browser_download_url -OutFile $path -UseBasicParsing

    $content = (Invoke-WebRequest -Uri $sum.browser_download_url -UseBasicParsing).Content
    if ($content -is [byte[]]) { $content = [Text.Encoding]::ASCII.GetString($content) }
    $expected = ($content.Trim() -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -Path $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($expected -ne $actual) { throw "Checksum mismatch (expected $expected, got $actual)." }
    Write-Host 'Checksum OK. Starting the installer...'

    Start-Process -FilePath $path -Wait
}
finally {
    Remove-Item -Path $dir -Recurse -Force -ErrorAction SilentlyContinue
}
