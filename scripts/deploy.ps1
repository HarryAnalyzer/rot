<#
.SYNOPSIS
  Build $ROT, deploy both cadences to NEAR testnet, and point the page at them.

.DESCRIPTION
  Two deployments of the identical wasm:

    rot.<master>       a day is a day       (the parameters the simulation chose)
    rot-demo.<master>  a day is one minute  (so the economy plays out while you watch)

  Nothing here is interactive except the keychain prompts from near-cli-rs.

.EXAMPLE
  ./scripts/deploy.ps1 -Master you.testnet

.EXAMPLE
  # Redeploy without recreating the sub-accounts
  ./scripts/deploy.ps1 -Master you.testnet -SkipCreate
#>
[CmdletBinding()]
param(
  # Your funded testnet account. The two contracts become sub-accounts of it.
  [Parameter(Mandatory = $true)][string]$Master,

  # Skip sub-account creation (they already exist).
  [switch]$SkipCreate,

  # Skip `cargo build` and use whatever wasm is already there.
  [switch]$SkipBuild,

  # NEAR this script funds each contract sub-account with. Storage costs
  # 1 NEAR per 100 KB of code, so 3 covers a ~250 KB wasm with room for the
  # account state that holders add as they register. The testnet faucet gives
  # 10 NEAR in total, and scripts/demo.ps1 needs a slice of it too.
  [string]$Fund = '3 NEAR',

  [string]$Network = 'testnet'
)

# near-cli writes its transaction summaries to stderr. Under 'Stop', PowerShell
# turns any native stderr line into a terminating NativeCommandError and kills
# the script mid-deploy, so every near call below is checked by $LASTEXITCODE
# instead.
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

function Step($msg) { Write-Host "`n=== $msg" -ForegroundColor Cyan }
function Ok($msg) { Write-Host "    $msg" -ForegroundColor Green }
function Note($msg) { Write-Host "    $msg" -ForegroundColor DarkGray }

if (-not (Get-Command near -ErrorAction SilentlyContinue)) {
  throw "near-cli-rs is not on PATH. Install it, then run this again."
}

$slow = "rot.$Master"
$demo = "rot-demo.$Master"

# --------------------------------------------------------------------------
# 1. Build
# --------------------------------------------------------------------------

if (-not $SkipBuild) {
  Step "Building the contract"
  & (Join-Path $PSScriptRoot 'build.ps1')
  if ($LASTEXITCODE -ne 0) { throw "build failed" }
}

# build.ps1 writes the optimised one; a plain `cargo build` leaves the larger
# one in target/. Either deploys, the first just costs half as much storage.
$wasm = @(
  (Join-Path $root 'build\rot.wasm'),
  (Join-Path $root 'target\wasm32-unknown-unknown\release\rot.wasm')
) | Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $wasm) {
  throw "No wasm found. Run ./scripts/build.ps1 first."
}
$size = [math]::Round((Get-Item $wasm).Length / 1KB, 1)
Ok "$wasm ($size KB)"

# --------------------------------------------------------------------------
# 2. Sub-accounts
# --------------------------------------------------------------------------

function New-SubAccount($name) {
  Step "Creating $name"
  near account create-account fund-myself $name $Fund `
    autogenerate-new-keypair save-to-keychain `
    sign-as $Master `
    network-config $Network sign-with-keychain send
  if ($LASTEXITCODE -ne 0) { throw "could not create $name" }
  Ok "$name created"
}

if (-not $SkipCreate) {
  New-SubAccount $slow
  New-SubAccount $demo
} else {
  Note "skipping account creation"
}

# --------------------------------------------------------------------------
# 3. Deploy
# --------------------------------------------------------------------------

# The whole supply goes into the faucet: no team allocation, no treasury, no
# presale. Every token in circulation was claimed by somebody.
$supply = '1' + ('0' * 27)   # 1,000,000,000 ROT at 18 decimals
$faucet = '1' + ('0' * 21)   # 1,000 ROT per account

function New-InitArgs($dayMs, $rotBps, $stampDays, $epochDays, $claimWindow) {
  $o = [ordered]@{
    name                = 'Rot'
    symbol              = 'ROT'
    icon                = 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><text y="26" font-size="26">&#129443;</text></svg>'
    day_ms              = $dayMs
    rot_bps_per_day     = $rotBps
    stamp_days          = $stampDays
    stamp_cost_bps      = 50
    epoch_days          = $epochDays
    claim_window_epochs = $claimWindow
    total_supply        = $supply
    faucet_amount       = $faucet
  }
  $o | ConvertTo-Json -Compress -Depth 5
}

function Deploy-Rot($account, $argsJson, $label) {
  Step "Deploying $label to $account"
  # The init args are handed over base64-encoded. Windows PowerShell does not
  # escape the double quotes inside a JSON string when it hands an argument to
  # a native executable, so `json-args` arrives at near-cli split into pieces
  # the moment the JSON contains a quoted value with a space in it. Base64 is
  # quote-free and space-free, so it survives the trip intact.
  $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($argsJson))
  near contract deploy $account `
    use-file $wasm `
    with-init-call new base64-args $b64 `
    prepaid-gas '100 TGas' attached-deposit '0 NEAR' `
    network-config $Network sign-with-keychain send
  if ($LASTEXITCODE -ne 0) { throw "deploy to $account failed" }
  Ok "$account is live"
}

# Real cadence: the parameters sim/RESULTS.md argued for.
Deploy-Rot $slow (New-InitArgs 86400000 200 7 7 8) 'the real cadence (2%/day, 7-day stamp)'

# Demo cadence: a day is a minute and rot is 10% of it, so a judge watching the
# page sees the balance fall, an epoch close and a payout land inside a few
# minutes. Same code, same rules, faster clock and a louder number.
Deploy-Rot $demo (New-InitArgs 60000 1000 3 3 4) 'the demo cadence (1 min/day, 10%/day)'

# --------------------------------------------------------------------------
# 4. Point the page at them
# --------------------------------------------------------------------------

Step "Updating web/config.js"
$cfgPath = Join-Path $root 'web\config.js'
$cfg = Get-Content $cfgPath -Raw
$cfg = $cfg -replace 'contract: "rot-demo\.[^"]*"', "contract: `"$demo`""
$cfg = $cfg -replace 'contract: "rot\.[^"]*"', "contract: `"$slow`""
Set-Content $cfgPath $cfg -Encoding utf8 -NoNewline
Ok "page now reads $demo and $slow"

# --------------------------------------------------------------------------
# 5. Check it answers
# --------------------------------------------------------------------------

Step "Asking both contracts whether their books balance"
foreach ($a in @($demo, $slow)) {
  near contract call-function as-read-only $a audit_supply json-args '{}' `
    network-config $Network now
}

Write-Host "`nDone." -ForegroundColor Green
Write-Host @"

  demo contract : $demo
  real contract : $slow
  page          : python -m http.server 8787 --directory web

Next, in order:

  1. ./scripts/demo.ps1 -Master $Master     run a full cycle on the demo contract
  2. record the page while that runs
  3. ./scripts/lock.ps1 -Master $Master     delete the keys, make it immutable

Do step 3 LAST. It cannot be undone, and after it nobody can ever redeploy
either contract - which is the point, but it does mean no more fixes.
"@ -ForegroundColor Gray

Pop-Location
