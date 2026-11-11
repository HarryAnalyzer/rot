<#
.SYNOPSIS
  Drive a whole $ROT cycle on the demo contract, start to finish, on camera.

.DESCRIPTION
  Three accounts, one story:

    stamper  claims from the faucet and stamps every cycle
    sleeper  claims once and then does nothing, ever
    poker    claims, and makes a living poking the sleeper

  The script claims, stamps, waits for the sleeper to rot, pokes it into the
  heap, waits for the epoch to close and then claims the payout - printing the
  balances at every step so the transfer of wealth from sleeper to stamper is
  visible in the terminal as well as on the page.

  Run it beside the open page. It takes about six minutes on the demo cadence.

.EXAMPLE
  ./scripts/demo.ps1 -Master you.testnet
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Master,
  [switch]$SkipCreate,
  [string]$Network = 'testnet',
  # Seconds per simulated day on the demo contract.
  [int]$DaySeconds = 60
)

$ErrorActionPreference = 'Stop'
$contract = "rot-demo.$Master"
# Named for what they do, because these ids are on screen in the demo.
$stamper = "stamper.$Master"
$sleeper = "sleeper.$Master"
$poker = "poker.$Master"

function Step($m) { Write-Host "`n=== $m" -ForegroundColor Cyan }
function Say($m) { Write-Host "    $m" -ForegroundColor Gray }

function Call($signer, $method, $json, $deposit = '0 NEAR', $gas = '100 TGas') {
  near contract call-function as-transaction $contract $method `
    json-args $json prepaid-gas $gas attached-deposit $deposit `
    sign-as $signer network-config $Network sign-with-keychain send
  if ($LASTEXITCODE -ne 0) { throw "$method failed for $signer" }
}

function View($method, $json = '{}') {
  $out = near contract call-function as-read-only $contract $method `
    json-args $json network-config $Network now 2>&1 | Out-String
  return $out
}

function Balance($who) {
  $raw = View 'ft_balance_of' (@{ account_id = $who } | ConvertTo-Json -Compress)
  if ($raw -match '"(\d+)"') {
    return [math]::Round([double]$Matches[1] / 1e18, 2)
  }
  return $null
}

function Show($label) {
  $s = Balance $stamper
  $l = Balance $sleeper
  $p = Balance $poker
  Write-Host ("    {0,-22} stamper {1,10}  sleeper {2,10}  poker {3,10}" -f $label, $s, $l, $p) -ForegroundColor White
}

function Wait-Days($n, $why) {
  $sec = $n * $DaySeconds + 5
  Say "waiting $sec s - $why"
  Start-Sleep -Seconds $sec
}

# --------------------------------------------------------------------------

if (-not $SkipCreate) {
  Step "Creating three accounts"
  # Half a NEAR each: they only ever pay a storage deposit and some gas.
  foreach ($a in @($stamper, $sleeper, $poker)) {
    near account create-account fund-myself $a '0.5 NEAR' `
      autogenerate-new-keypair save-to-keychain sign-as $Master `
      network-config $Network sign-with-keychain send
    if ($LASTEXITCODE -ne 0) { throw "could not create $a" }
    Say "$a"
  }
}

Step "Everyone takes their one share from the faucet"
foreach ($a in @($stamper, $sleeper, $poker)) {
  Call $a 'faucet_claim' '{}' '0.00125 NEAR'
}
Show 'after the faucet'

Step "The stamper stamps - burns 0.5%, raises a shield, buys into next epoch"
Call $stamper 'stamp' '{}'
Call $poker 'stamp' '{}'
Show 'after stamping'
Say "the sleeper did nothing, and that is the whole point"

Wait-Days 2 "letting the sleeper rot"
Show 'two days later'
Say "the sleeper fell, the stamper did not: that is the shield"

Step "Poking the sleeper - its rot drops into this epoch's compost heap"
Call $poker 'poke' (@{ account_ids = @($sleeper) } | ConvertTo-Json -Compress)
Write-Host (View 'get_stats') -ForegroundColor DarkGray

Wait-Days 2 "waiting for the epoch to close"

Step "Claiming the compost"
Call $stamper 'claim' '{}'
Call $poker 'claim' '{}'
Show 'after the payout'

Step "Do the books still balance?"
Write-Host (View 'audit_supply') -ForegroundColor Green

Write-Host @"

That is the whole mechanic:

  the sleeper lost tokens by doing nothing,
  the stamper gained them by paying 0.5% and showing up,
  and nobody minted anything to make it happen.

"@ -ForegroundColor Gray
