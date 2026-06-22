param(
    [switch]$SkipTests
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

function Warn($Message) {
    Write-Host "WARN $Message" -ForegroundColor Yellow
}

function Need($Command, $InstallHint) {
    if (-not (Get-Command $Command -ErrorAction SilentlyContinue)) {
        Warn "$Command was not found on PATH"
        if ($InstallHint) {
            Write-Host "  $InstallHint"
        }
        return $false
    }
    return $true
}

function Run($Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

Step "toolchain"
if (-not (Need "cargo" "Install Rust with rustup: https://rustup.rs")) {
    exit 2
}
Run "rustc" @("--version")
Run "cargo" @("--version")

$CargoToml = Get-Content Cargo.toml
$Msrv = ($CargoToml | Select-String '^rust-version\s*=\s*"([^"]+)"').Matches.Groups[1].Value
if ($Msrv) {
    Write-Host "workspace MSRV: $Msrv"
}

Step "native build prerequisites"
if (Test-Path "crates/leyline-bssl-sys/native/x86_64-pc-windows-msvc/lib/ssl.lib") {
    Write-Host "Windows prebuilt BoringSSL shim found; CMake/Perl only needed to refresh it."
} else {
    Need "cmake" "Install Visual Studio Build Tools with C++ CMake tools, or install CMake separately." | Out-Null
    Need "perl" "Install Strawberry Perl: choco install strawberryperl" | Out-Null
}
Ok "prerequisite scan complete"

Step "fast offline build"
Run "cargo" @("check", "-p", "leyline", "--all-features")
Ok "leyline all-features check passed"

if (-not $SkipTests) {
    Step "offline smoke tests"
    Run "cargo" @("test", "-p", "leyline", "--test", "parity_builder")
    Run "cargo" @("test", "-p", "leyline", "--test", "tls_happy_eyeballs")
    Ok "developer setup looks ready"
}

Write-Host ""
Write-Host "Next useful commands:"
Write-Host "  ./scripts/verify.sh --quick"
Write-Host "  cargo test --workspace --exclude leyline-quiche"
Write-Host "  cargo test -p leyline --test tls_peet -- --ignored"
