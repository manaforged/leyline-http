param(
    [string]$Target = ((rustc -vV | Select-String '^host: ').Line -replace '^host: ', '')
)

$ErrorActionPreference = "Stop"
$Package = "leyline-bssl-sys"
$Generated = "bindings.rs"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$DestDir = Join-Path $RepoRoot "crates/$Package/bindings"
Set-Location $RepoRoot

$ErrorActionPreference = "Continue"
$Messages = & cargo build --manifest-path "crates/$Package/Cargo.toml" --features bindgen --target $Target --message-format=json 2>$null
$ErrorActionPreference = "Stop"
if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed for $Target"
}

$OutDir = $Messages |
    ForEach-Object { $_ | ConvertFrom-Json } |
    Where-Object { $_.reason -eq "build-script-executed" -and $_.package_id -like "*$Package*" } |
    Select-Object -Last 1 -ExpandProperty out_dir

$Source = if ($OutDir) { Join-Path $OutDir $Generated }
if (-not $Source -or -not (Test-Path $Source)) {
    throw "no generated $Generated for $Target"
}

New-Item -ItemType Directory -Force -Path $DestDir | Out-Null
Copy-Item $Source (Join-Path $DestDir "$Target.rs")
Write-Host "wrote crates/$Package/bindings/$Target.rs"
