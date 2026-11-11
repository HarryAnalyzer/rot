<#
.SYNOPSIS
  Delete every access key from the $ROT contract accounts, for good.

.DESCRIPTION
  $ROT has no owner, no mint, no pause and no setter. This is the step that
  makes that claim true rather than polite: with no full-access keys on the
  account, nobody - including whoever deployed it - can ever redeploy the code
  or change a parameter.

  It cannot be undone. Deploy, test and record your demo FIRST.

.EXAMPLE
  ./scripts/lock.ps1 -Master you.testnet
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Master,
  [string]$Network = 'testnet',
  # Lock only one of the two.
  [ValidateSet('both', 'demo', 'real')][string]$Which = 'both',
  # Skip the confirmation prompt.
  [switch]$Yes
)

# near-cli writes to stderr even when it succeeds; under 'Stop' that would
# abort the script. Each call checks $LASTEXITCODE instead.
$ErrorActionPreference = 'Continue'

$targets = @()
if ($Which -in @('both', 'real')) { $targets += "rot.$Master" }
if ($Which -in @('both', 'demo')) { $targets += "rot-demo.$Master" }

Write-Host "`nAbout to permanently remove every access key from:" -ForegroundColor Yellow
$targets | ForEach-Object { Write-Host "  $_" -ForegroundColor Yellow }
Write-Host @"

After this the contracts are immutable. No redeploy, no parameter change, no
recovery - not by you, not by anyone. That is the intended end state, but if
you have not finished testing, stop here.
"@ -ForegroundColor Yellow

if (-not $Yes) {
  $answer = Read-Host "Type LOCK to continue"
  if ($answer -ne 'LOCK') {
    Write-Host "Nothing was changed." -ForegroundColor Green
    exit 0
  }
}

foreach ($account in $targets) {
  Write-Host "`n=== $account" -ForegroundColor Cyan

  $raw = near account list-keys $account network-config $Network now 2>&1 | Out-String
  $keys = [regex]::Matches($raw, 'ed25519:[1-9A-HJ-NP-Za-km-z]{40,50}') |
    ForEach-Object { $_.Value } |
    Select-Object -Unique

  if (-not $keys) {
    Write-Host "    no keys found - already locked" -ForegroundColor Green
    continue
  }

  Write-Host "    deleting $($keys.Count) key(s)" -ForegroundColor Gray
  # Deleting the signing key last, so the earlier deletions can still be signed.
  near account delete-keys $account public-keys @keys `
    network-config $Network sign-with-keychain send
  if ($LASTEXITCODE -ne 0) { throw "could not delete keys on $account" }

  Write-Host "    locked" -ForegroundColor Green
}

Write-Host "`nVerify with:" -ForegroundColor Gray
$targets | ForEach-Object {
  Write-Host "  near account list-keys $_ network-config $Network now" -ForegroundColor Gray
}
Write-Host "An empty key list is the proof that nothing above can change." -ForegroundColor Gray
