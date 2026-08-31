param(
    [switch]$Quick
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RepoRoot

function Step($Message) {
    Write-Host ""
    Write-Host "== $Message ==" -ForegroundColor Blue
}

function Ok($Message) {
    Write-Host "OK $Message" -ForegroundColor Green
}

function Run($Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

Step "rust comments (one line)"
$py = Get-Command python3 -ErrorAction SilentlyContinue
if (-not $py) { $py = Get-Command python -ErrorAction SilentlyContinue }
if (-not $py) { throw "python3 not installed" }
Run $py.Source @("scripts/check-comments.py")
Ok "comment lint"

Step "rust toolchain"
Run "rustc" @("--version")
Run "cargo" @("--version")

Step "cargo fmt --all --check"
Run "cargo" @("fmt", "--all", "--check")
Ok "format clean"

Step "cargo clippy"
Run "cargo" @(
    "clippy",
    "--workspace",
    "--exclude",
    "leyline-quiche",
    "--all-targets",
    "--no-deps",
    "--",
    "-D",
    "warnings"
)
Ok "clippy clean"

Step "cargo doc"
$env:RUSTDOCFLAGS = "-D warnings"
Run "cargo" @("doc", "--workspace", "--exclude", "leyline-quiche", "--no-deps")
Ok "docs clean"

Step "cargo test --workspace --exclude leyline-quiche"
$env:RUSTDOC = (& rustup which rustdoc)
Run "cargo" @("test", "--workspace", "--exclude", "leyline-quiche")
Ok "tests pass"

if (-not $Quick) {
    Step "live tls_peet"
    Run "cargo" @("test", "-p", "leyline-http", "--test", "tls_peet", "--", "--ignored")
    Ok "live tls_peet pass"

    Step "live smoke"
    Run "cargo" @("test", "-p", "leyline-http", "--test", "smoke", "--", "--ignored", "--nocapture")
    Ok "smoke pass"
}

Step "cargo deny"
if (-not (Get-Command cargo-deny -ErrorAction SilentlyContinue)) {
    throw "cargo-deny not installed. Install with: cargo install --locked cargo-deny"
}
Run "cargo" @("deny", "--all-features", "check")
Ok "cargo-deny clean"

Write-Host ""
Write-Host "All verify gates passed." -ForegroundColor Green
