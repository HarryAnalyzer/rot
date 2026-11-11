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
  [int]$DaySeconds = 60,
  # Suffix for the three actor accounts. The faucet pays each account exactly
  # once, so a rehearsal run and the run you record need different names if you
  # want both to start from a full balance.
  [string]$Tag = '',
  [string]$Fund = '0.5 NEAR'
)

# near-cli writes to stderr even when it succeeds; under 'Stop' that would
# abort the script. Each call checks $LASTEXITCODE instead.
$ErrorActionPreference = 'Continue'
$contract = "rot-demo.$Master"
# Named for what they do, because these ids are on screen in the demo.
$suffix = if ($Tag) { "-$Tag" } else { '' }
$stamper = "stamper$suffix.$Master"
$sleeper = "sleeper$suffix.$Master"
$poker = "poker$suffix.$Master"

function Step($m) { Write-Host "`n=== $m" -ForegroundColor Cyan }
function Say($m) { Write-Host "    $m" -ForegroundColor Gray }

# Args go over as base64: Windows PowerShell mangles the quotes in a JSON
# argument on its way to a native executable.
function B64($json) {
  return [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($json))
}

function Call($signer, $method, $json, $deposit = '0 NEAR', $gas = '100 TGas') {
  near contract call-function as-transaction $contract $method `
    base64-args (B64 $json) prepaid-gas $gas attached-deposit $deposit `
    sign-as $signer network-config $Network sign-with-keychain send
  if ($LASTEXITCODE -ne 0) { throw "$method failed for $signer" }
}

function View($method, $json = '{}') {
  $out = near contract call-function as-read-only $contract $method `
    base64-args (B64 $json) network-config $Network now | Out-String
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

# Each transaction costs ten to fifteen seconds, and on the demo contract a
# whole day is sixty. Sleeping for a fixed number of seconds therefore drifts
# days behind within one run, and a shield that lapses unnoticed ruins the
# story. So the script asks the contract what day it is and waits for the one
# it actually wants.
function Current-Day {
  $raw = View 'get_stats'
  if ($raw -match '"day":\s*(\d+)') { return [int]$Matches[1] }
  throw "could not read the current day from get_stats"
}

function Wait-ToDay($target, $why) {
  Say "waiting for day $target - $why"
  while ($true) {
    $d = Current-Day
    if ($d -ge $target) { return $d }
    Start-Sleep -Seconds 4
  }
}

# --------------------------------------------------------------------------

if (-not $SkipCreate) {
  Step "Creating three accounts"
  # Half a NEAR each: they only ever pay a storage deposit and some gas.
  foreach ($a in @($stamper, $sleeper, $poker)) {
    near account create-account fund-myself $a $Fund `
      autogenerate-new-keypair save-to-keychain sign-as $Master `
      network-config $Network sign-with-keychain send
    if ($LASTEXITCODE -ne 0) { throw "could not create $a" }
    Say "$a"
  }
}

# Everything below is timed off epoch boundaries. A stamp bought in epoch N
# earns from epoch N+1, so the run has to start at the top of an epoch for the
# poke to land in the same heap the stampers have weight in.
$epochDays = 3
Step "Lining up with the start of an epoch"
$day = Current-Day
$d0 = [math]::Ceiling(($day + 1) / $epochDays) * $epochDays
Say "it is day $day; the next epoch opens on day $d0"
$null = Wait-ToDay $d0 "the top of an epoch"

Step "Everyone takes their one share from the faucet"
foreach ($a in @($stamper, $sleeper, $poker)) {
  # The faucet pays each account once, so on a repeat run this call is
  # expected to fail. Carry on with whatever balance they still hold.
  near contract call-function as-transaction $contract faucet_claim `
    base64-args (B64 '{}') prepaid-gas '100 TGas' attached-deposit '0.00125 NEAR' `
    sign-as $a network-config $Network sign-with-keychain send
  if ($LASTEXITCODE -ne 0) { Say "$a had already claimed - continuing" }
}
Show 'after the faucet'

Step "Two of them stamp - burn 0.5%, raise a shield, buy into the next epoch"
Call $stamper 'stamp' '{}'
Call $poker 'stamp' '{}'
Show 'after stamping'
Say "the sleeper did nothing, and that is the whole point"

$null = Wait-ToDay ($d0 + 2) "letting the sleeper rot"
Show 'two days later'
Say "the sleeper fell, the stamped two did not: that is the shield"

# A shield only lasts stamp_days, and the epoch you earn from begins after the
# one you stamped in - so staying ahead means stamping every single epoch. Skip
# one and you rot through the very epoch you are owed compost from, which is
# the whole reason the shield and the epoch are the same length.
$null = Wait-ToDay ($d0 + $epochDays) "the epoch they bought into"
Step "New epoch. They stamp again - a shield only covers the days it bought"
Call $stamper 'stamp' '{}'
Call $poker 'stamp' '{}'

Step "Poking the sleeper - its rot drops into this epoch's compost heap"
Call $poker 'poke' (@{ account_ids = @($sleeper) } | ConvertTo-Json -Compress)
Write-Host (View 'get_stats') -ForegroundColor DarkGray

$null = Wait-ToDay ($d0 + 2 * $epochDays) "waiting for that epoch to close"

Step "One more stamp to stay covered, then they collect"
Call $stamper 'stamp' '{}'
Call $poker 'stamp' '{}'
Call $stamper 'claim' '{}'
Call $poker 'claim' '{}'
Show 'after the payout'

Step "Do the books still balance?"
Write-Host (View 'audit_supply') -ForegroundColor Green

Write-Host @"

That is the whole mechanic:

  the sleeper lost tokens by doing nothing,
  the stampers took them by paying 0.5% and showing up every epoch,
  and nobody minted anything to make it happen.

"@ -ForegroundColor Gray
