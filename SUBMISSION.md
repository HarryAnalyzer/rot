# Submission checklist — NEAR HACKATHON

Deadline: **14 Nov 2026, 23:59 UTC** (03:29 Saturday 15 Nov, Tehran time).
Track: **11 — Memecoins With Mechanics** (`memecoins`).

Fill the blanks marked `<…>` as you go. Nothing here is posted or submitted
automatically — every outward step is yours.

---

## 1. Before you post

- [ ] `cargo test` passes
- [ ] `python sim/simulate.py` runs and `sim/RESULTS.md` is up to date
- [ ] both contracts deployed, `audit_supply` returns `true` on each
- [ ] `web/config.js` points at the deployed account ids
- [ ] `scripts/demo.ps1` ran end to end at least once
- [ ] repo is **public** on GitHub with the LICENSE file
- [ ] `scripts/lock.ps1` run **last**, keys deleted, `list-keys` returns empty

Filled in:

- master account: `worgl.testnet`
- real contract: `rot.worgl.testnet` — 2%/day, 7-day stamp, 7-day epochs
- demo contract: `rot-demo.worgl.testnet` — 10%/day, 1 minute per day
- repo: <https://github.com/HarryAnalyzer/rot>
- page: served from `web/`; `python -m http.server 8787 --directory web`

Verified on 12 Nov 2026: 72/72 tests pass, both contracts deployed,
`audit_supply` returns `true` on each, and a full faucet → stamp → rot → poke
→ claim cycle ran live on the demo contract. A stamper who kept up ended at
98.6% of their starting balance; the sleeper beside them fell to 53%.

---

## 2. The demo video

60–90 seconds, screen recording, no intro, no architecture slides. Run
`scripts/demo.ps1 -Master worgl.testnet` beside the open page and record both.

The script aligns itself to an epoch boundary and then waits on the contract's
own clock, so a full cycle takes **8–11 minutes** of wall time. Record the
whole thing and cut it down, or record the four moments below. The long waits
are where the page earns its keep: the countdown runs, and balances drop on
their own.

The beat sheet:

1. **(0:00)** The page, on the demo contract. Point at one number: a balance
   falling on its own, with the "next rot tick" countdown beside it. Say: *this
   is a token balance, and nobody is sending transactions.*
2. **(0:12)** Two accounts side by side — one stamped, one not. The stamped one
   holds still. The other drops every tick.
3. **(0:30)** Poke the sleeper. Its rot lands in the epoch's compost heap, live
   in the event log.
4. **(0:45)** The epoch closes. The stamper claims. Their balance goes **up**,
   and no tokens were minted — the sleeper paid for it.
5. **(1:00)** `audit_supply` → `true`. Then `near account list-keys` on the
   contract → empty. Say: *no owner, no mint, no pause, and now no keys.*
6. **(1:10)** The fast-forward playground: drag participation to 10%, watch the
   two lines separate over sixty days.

Record it, then upload to X directly (video, not a link).

---

## 3. The X post

Must say you built it at NEAR HACKATHON, tag **@NEARProtocol**, use
**#NEARHackathon**, and carry the video.

```
Your memecoin's supply is a number. $ROT's balance is a function of the clock.

Leave it alone and it loses 2% a day — every token that rots is paid to
the holders who stamped theirs that week. No mint, no owner, no keys.

Austria tried this in 1932. It worked, so they banned it.

Built at NEAR HACKATHON @NEARProtocol #NEARHackathon
<repo link>
```

Alternates, if the first feels too long:

```
I made a memecoin you can lose by doing nothing.

$ROT drops 2% a day unless you stamp it, and everything that rots goes to
the people who did. The contract has no owner and no access keys — the
rules cannot change, including by me.

Built at NEAR HACKATHON @NEARProtocol #NEARHackathon
```

```
In 1932 a town in Austria issued money that expired unless you bought a
stamp for it. Unemployment fell 25% in a year. The central bank banned it.

$ROT is that, on NEAR, as a memecoin. Your balance rots. Stampers eat it.

Built at NEAR HACKATHON @NEARProtocol #NEARHackathon
```

---

## 4. The form

At <https://nearhackathon.com> → Apply. Fields, ready to paste:

**Project name**

```
$ROT — the coin that spoils
```

**Tracks**: `Memecoins With Mechanics` (primary). Optionally also `Open
Innovation`.

> Reviewed the 581 submissions already in: memecoins is the least crowded track
> (39), and nothing in it uses demurrage. Adding more tracks does not add prize
> money and dilutes the pitch — one track, aimed squarely.

**NEAR wallet address**: `<your mainnet account>` — this is where a prize would
be paid. A **mainnet** account, not the testnet one.

**Repository**: `https://github.com/<user>/rot` (public)

**Demo post**: the X link from step 3

**Description** (20–2,000 chars):

```
$ROT is a NEP-141 memecoin whose balance is a function of the clock. An
unstamped balance loses 2% of itself every day, compounding, and ft_balance_of
returns the rotted figure — no keeper, no cron, no transaction needed for your
balance to fall.

Everything that rots is paid out. stamp() burns 0.5% of your balance, shields
the rest for seven days, and registers your weight in the NEXT epoch, so an
epoch's weight is sealed before any compost lands in it and there is nothing to
front-run. Anyone can poke anyone: rot only moves when an account is settled,
which makes poking sleepers profitable and the economy self-driving. If nobody
stamped for an epoch, its rot burns instead. Supply can only shrink.

It is the Wörgl experiment of 1932 — scrip that lost value unless you bought a
stamp for it — as a memecoin, with the contract as the town clerk.

The mechanic lives entirely in the contract: no owner, no mint, no pause, no
setter, and the account's access keys are deleted, so not even the deployer can
change a parameter. The whole ruleset is ~150 lines of pure arithmetic in
src/math.rs, shared by the contract, a stdlib-only 60-day simulation across five
scenarios, and a browser model that lets the token page fast-forward the economy
without touching a chain. Shared test vectors keep all three honest; cargo test
asserts them, and the page shows a badge when its model agrees.

Sixty simulated days, half the holders stamping: stampers +46.9%, sleepers
−70.2%, supply −8.1%. With nobody stamping at all, nothing breaks — the rot just
burns. There is no state in which this token needs rescuing.

Sybil note, plainly: the demo faucet is farmable, but the mechanism is not —
rot and rewards are both strictly proportional to balance, so N wallets holding
X/N behave exactly like one wallet holding X. There is a test for it.
```

**Team size**: 1

---

## 5. After submitting

- [ ] Open the repo link in a private window — is it really public?
- [ ] Open the X post in a private window — does the video play?
- [ ] Clone fresh into an empty folder, run `cargo test` and
      `python sim/simulate.py`. The first screening is Claude Code and Codex
      checking that the project runs and does what the README says; a README
      that doesn't reproduce is the most common way to lose before a human
      ever sees it.
- [ ] Corrections go through nearhackathon.com/request-changes — not a second
      submission.
