# Submission checklist — NEAR HACKATHON

Deadline: **14 Nov 2026, 23:59 UTC** (03:29 Saturday 15 Nov, Tehran time).
Track: **11 — Memecoins With Mechanics** (`memecoins`).

Fill the blanks marked `<…>` as you go. Nothing here is posted or submitted
automatically — every outward step is yours.

---

## 1. Before you post

- [x] `cargo test` passes — 72/72, also green in CI on a clean runner
- [x] `python sim/simulate.py` runs and `sim/RESULTS.md` is up to date — CI fails if not
- [x] both contracts deployed, `audit_supply` returns `true` on each
- [x] `web/config.js` points at the deployed account ids
- [x] `scripts/demo.ps1` ran end to end — four times, one of them recorded
- [x] repo is **public** on GitHub with the LICENSE file
- [x] keys deleted on both contracts; two RPC endpoints report 0 access keys

Filled in:

- master account: `worgl.testnet`
- real contract: `rot.worgl.testnet` — 2%/day, 7-day stamp, 7-day epochs
- demo contract: `rot-demo.worgl.testnet` — 10%/day, 1 minute per day
- repo: <https://github.com/HarryAnalyzer/rot>
- page: <https://harryanalyzer.github.io/rot/>
- video: `rot-demo-final.mp4` on the Desktop — 76.8 s, 1080p, captioned, scored

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

A standard X account allows **280 characters**, and every link counts as 23.
This one is 272:

```
$ROT: a memecoin you lose by holding.

Leave it alone and it rots 2% a day. Stamp it and you collect what others lose. No owner, no keys.

Austria tried it in 1932. It worked, so they banned it.

Built at NEAR HACKATHON @NEARProtocol #NEARHackathon
github.com/HarryAnalyzer/rot
```

Longer alternates, for an account with X Premium:

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
$ROT is a NEP-141 memecoin whose balance rots. An unstamped balance loses 2% of itself every day, and ft_balance_of already returns the rotted figure — no keeper, no cron, no transaction needed.

Rot is not destroyed, it is redistributed. stamp() burns 0.5% of your balance, shields it for 7 days, and registers you for the next epoch's compost heap. Everything that rots goes to the holders who stamped, pro rata. Anyone can poke a sleeper to push its rot into the heap. If nobody stamped, the rot burns. Supply can only shrink.

It is the Wörgl experiment of 1932 — money that lost value unless you bought a stamp for it — with the contract as the town clerk.

No owner, no mint, no pause, no setters. Both contracts on testnet have zero access keys, so not even the deployer can change a rule (proof in the README). Splitting tokens across wallets gains nothing: rot and payouts are both proportional to balance.

The whole ruleset is ~150 lines in src/math.rs. 72 tests, CI green on every push, and a 60-day simulation: at 2% a day with half the holders stamping, stampers +46.9%, sleepers −70.2%.

Live page: https://harryanalyzer.github.io/rot/
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
