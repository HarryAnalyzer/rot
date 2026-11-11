<#
.SYNOPSIS
  Build the $ROT wasm, with nothing installed but Docker.

.DESCRIPTION
  Compiles in the official rust image and shrinks the result with wasm-opt,
  leaving build/rot.wasm. Storage on NEAR costs 1 NEAR per 100 KB of code, so
  the -Oz pass is not housekeeping: it halves the deployment bill.

  If you have Rust locally, you do not need this at all:

    cargo build --target wasm32-unknown-unknown --release

  produces the same contract at target/wasm32-unknown-unknown/release/rot.wasm,
  just larger, because it skips wasm-opt.

.EXAMPLE
  ./scripts/build.ps1
#>
[CmdletBinding()]
param(
  # Skip wasm-opt; emit the raw cargo output.
  [switch]$NoOptimise,
  [string]$Image = 'rust:1.99'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
  throw "Docker is not on PATH. Either install it, or build with cargo directly."
}
docker info 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Docker is installed but not running." }

# wasm-opt is told which features the input already uses. It does not add any:
# modern rustc emits bulk-memory and sign-ext, and the Debian wasm-opt refuses
# to read them unless asked.
$features = '--enable-bulk-memory --enable-sign-ext --enable-mutable-globals --enable-nontrapping-float-to-int'

$optimise = if ($NoOptimise) {
  'cp target/wasm32-unknown-unknown/release/rot.wasm /w/build/rot.wasm'
} else {
  "apt-get update -qq >/dev/null 2>&1; apt-get install -y -qq binaryen >/dev/null 2>&1; " +
  "wasm-opt -Oz $features target/wasm32-unknown-unknown/release/rot.wasm -o /w/build/rot.wasm"
}

$script = @"
set -e
rustup target add wasm32-unknown-unknown >/dev/null 2>&1
cargo build --target wasm32-unknown-unknown --release
mkdir -p /w/build
$optimise
ls -l /w/build/rot.wasm
"@

Write-Host "Building in $Image ..." -ForegroundColor Cyan

# Named volumes keep the crate registry and build cache off the Windows drive,
# where the bind mount makes cargo an order of magnitude slower.
docker run --rm `
  -v "${root}:/w" -w /w `
  -v rot-cargo:/usr/local/cargo/registry `
  -v rot-target:/w/target `
  -e CARGO_TERM_COLOR=never -e RUSTUP_TOOLCHAIN=stable `
  $Image bash -c $script

if ($LASTEXITCODE -ne 0) { throw "build failed" }

$wasm = Join-Path $root 'build\rot.wasm'
$kb = [math]::Round((Get-Item $wasm).Length / 1KB, 1)
$near = [math]::Round((Get-Item $wasm).Length / 100KB, 2)

Write-Host "`n  build/rot.wasm  $kb KB  (~$near NEAR of storage per deployment)" -ForegroundColor Green
Write-Host "  next: ./scripts/deploy.ps1 -Master <you>.testnet`n" -ForegroundColor Gray
