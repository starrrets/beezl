# Writes the manifest templates, filled in for a published release, to -Out.
# The checksum is taken from the published file.
param([Parameter(Mandatory)][string]$Version, [Parameter(Mandatory)][string]$Out)

New-Item -ItemType Directory -Force $Out | Out-Null
$setup = Join-Path ([IO.Path]::GetTempPath()) "beezl-setup-$Version.exe"
Invoke-WebRequest "https://github.com/starrrets/beezl/releases/download/v$Version/beezl-setup.exe" -OutFile $setup
$sha = (Get-FileHash $setup -Algorithm SHA256).Hash
Remove-Item $setup
Get-ChildItem "$PSScriptRoot\*.yaml" | ForEach-Object {
    (Get-Content $_ -Raw).Replace('{{VERSION}}', $Version).Replace('{{SHA256}}', $sha) |
        Set-Content -NoNewline -Encoding utf8NoBOM (Join-Path $Out $_.Name)
}
