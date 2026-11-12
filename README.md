# $ROT — the coin that spoils

[![tests](https://github.com/HarryAnalyzer/rot/actions/workflows/ci.yml/badge.svg)](https://github.com/HarryAnalyzer/rot/actions/workflows/ci.yml)
[![token page](https://img.shields.io/badge/token%20page-live-16a34a)](https://harryanalyzer.github.io/rot/)

**A NEP-141 memecoin on NEAR whose balance falls on its own, and pays the
holders who didn't let theirs.**

An untouched balance of $ROT loses 2% of itself every day. Everything that rots
is paid out to the holders who **stamped** their tokens that week. Stop
participating and your balance quietly drains into the pockets of the people
who did.

`ft_balance_of` is a function of the clock. Nobody has to send a transaction for
your balance to fall — it is already lower.

> Built at NEAR HACKATHON. Track: **Memecoins With Mechanics**.

---

## The joke has a footnote

In 1932 the Austrian town of Wörgl issued scrip that lost 1% of its value each
month unless the holder bought a stamp and stuck it to the note. Hoarding became
expensive, the notes changed hands roughly fourteen times faster than schillings,
and a town with 30% unemployment built a bridge, a ski jump and a drainage
system in a year. Economists call it *demurrage*; Irving Fisher wrote a book
about it and Keynes gave it a paragraph. Austria's central bank shut it down in
1933 and the idea has been a footnote ever since.

$ROT is that footnote, as a memecoin, with the stamp enforced by a contract
instead of a town clerk.

---

## The whole mechanic

Five verbs. No admin.

| | |
|---|---|
| **Rot** | An unstamped balance loses `rot_bps_per_day` of itself per day, compounding. No keeper, no cron: the balance *is* the formula. |
| **Stamp** | `stamp()` burns 0.5% of your balance, shields the rest from rot for 7 days, and registers your balance as **weight in the next epoch**. Stamping is the only way to earn. |
| **Compost** | Rot does not vanish. When an account is settled, its rot drops into the current epoch's heap, and the holders who stamped for that epoch split it pro rata. If nobody stamped for that epoch, it **burns** instead. |
| **Poke** | Rot only *moves* when an account is settled, and **anyone may settle anyone**. If you are stamped this epoch, poking the sleepers is how you get paid. |
| **Claim** | Collect your cut of every finished epoch. Your payout is capped by what you **still hold**. |

### Why earning starts *next* epoch

The one real attack on any stake-weighted payout is to watch the pot grow and
buy in at the end. $ROT closes it by construction: `stamp()` shields you from
**today**, but registers your weight for the **next** epoch. So by the time an
epoch starts collecting compost, its weight is already final and sealed. There
is no front-run, because there is nothing left to front-run.

The shield lasting exactly as long as an epoch is what ties it together: stamp
once a week and you are always shielded *and* always earning. Miss one and you
are neither.

### Supply can only shrink

Three sinks, no faucet beyond the launch supply, and no mint function at all:

1. every `stamp()` burns 0.5%,
2. rot in an epoch nobody stamped for burns,
3. compost nobody claims within 8 epochs can be burned by anyone via `sweep()`.

---

## Try it in 60 seconds

No signup, no wallet connect, nothing to install: open
**<https://harryanalyzer.github.io/rot/>**.

You get live contract state read straight off NEAR, and a **fast-forward
playground** that runs sixty days of the economy in your browser using the same
arithmetic the contract runs — drag the sliders and watch who ends up with the
tokens.

Two links worth opening first:

- **[What doing nothing costs](https://harryanalyzer.github.io/rot/?account=sleeper.worgl.testnet)**
  — the account from the demo video, which took 1,000 ROT from the faucet and
  never touched it again. On the demo contract a day lasts a minute, so it has
  been rotting at 10% a day ever since: within hours of the recording there was
  almost nothing left. No transaction did that. The clock did.
- **[The page on autopilot](https://harryanalyzer.github.io/rot/?tour)** — `?tour`
  walks through every section on its own; it is how the demo video was filmed,
  with nobody at the mouse.

The same page runs from the repository with no network but the chain's:

```bash
python -m http.server 8787 --directory web
```

To read the real thing straight off the chain, with nothing installed but
`curl`:

```bash
curl -s https://test.rpc.fastnear.com -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"query","params":{"request_type":"call_function","finality":"final","account_id":"rot-demo.worgl.testnet","method_name":"get_stats","args_base64":"e30="}}' | python -c "import json,sys;print(bytes(json.load(sys.stdin)['result']['result']).decode())"
```

> **Deployed contracts, both on testnet.**
> [`rot.worgl.testnet`](https://testnet.nearblocks.io/address/rot.worgl.testnet)
> runs at one day per day — the cadence the simulation tuned.
> [`rot-demo.worgl.testnet`](https://testnet.nearblocks.io/address/rot-demo.worgl.testnet)
> runs the *same wasm* at one minute per day, so the whole cycle — rot, epoch
> close, payout — plays out while you watch.

---

## Reproduce everything

Three commands, three different checks. Each is self-contained.

```bash
cargo test
```
Runs the full suite: the pure math, the mechanism, NEP-141 conformance, the
anti-exploit cases, and a 60-day run that asserts `audit_supply()` after every
single day. No network, no sandbox, no keys.

```bash
python sim/simulate.py
```
Replays five 60-day scenarios and writes [`sim/RESULTS.md`](sim/RESULTS.md) with
charts. Stdlib only — no pip install. It asserts the shared vectors in
`sim/vectors.json`, which `cargo test` also asserts, so the Rust, the Python and
the browser model cannot silently drift apart. (The page shows a green badge
when its model agrees; red if it doesn't.)

```bash
cargo build --target wasm32-unknown-unknown --release
```
Produces `target/wasm32-unknown-unknown/release/rot.wasm` (261 KB).

No Rust installed? `./scripts/build.ps1` does the same inside the official
`rust:1.99` image and runs `wasm-opt -Oz` over the result, which halves it to
**214 KB**. That matters more than it sounds: NEAR charges 1 NEAR of storage
per 100 KB of code, so the optimiser is worth about 0.5 NEAR per deployment.

### Deploying your own

```powershell
./scripts/deploy.ps1 -Master worgl.testnet   # build, create sub-accounts, deploy both, patch the page
./scripts/demo.ps1   -Master worgl.testnet   # drive a full cycle: faucet, stamp, rot, poke, claim
./scripts/lock.ps1   -Master worgl.testnet   # delete every access key — irreversible
```

`lock.ps1` is what turns "no admin" from a claim into a fact. Run it last.

### Both deployed contracts are locked

The last access key on each contract account was deleted on 12 Nov 2026, after
the demo was recorded:

| contract | key deletion | access keys now |
|---|---|---|
| `rot.worgl.testnet` | [5MoMYBXg…](https://testnet.nearblocks.io/txns/5MoMYBXgV39Y42MVu6utg7bGXz8TSfQKucfP1utEV76y) | 0 |
| `rot-demo.worgl.testnet` | [7GGqm8wG…](https://testnet.nearblocks.io/txns/7GGqm8wGAaNn7yw2PPejtsQF5jGpcmBjbMg77hqrxwLu) | 0 |

Nobody can redeploy either contract or change a parameter — including us.
Don't take that from this table; ask the chain:

```bash
curl -s https://rpc.testnet.near.org -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"query","params":{"request_type":"view_access_key_list","finality":"final","account_id":"rot.worgl.testnet"}}'
```

An empty `keys` list is the proof.

---

## What the simulation decided

The parameters were not picked for vibes. Five scenarios, sixty days, ten
holders, a rational poker settling idle accounts daily:

| scenario | supply after 60d | a stamper | a sleeper |
| --- | --- | --- | --- |
| Nobody ever stamps | −70.24% | — | 1,000 → 298 ROT (−70.2%) |
| Everyone stamps, every epoch | −3.78% | 1,000 → 936 ROT (−6.4%) | — |
| **Half awake — five stamp, five sleep** | **−8.08%** | **1,000 → 1,469 ROT (+46.9%)** | **1,000 → 298 ROT (−70.2%)** |
| One 100× whale never stamps | −11.64% | 1,000 → 6,848 ROT (+584.8%) | 100,000 → 29,755 ROT (−70.2%) |
| Two dead epochs, then everyone wakes | −27.15% | 1,000 → 713 ROT (−28.7%) | — |

Every figure above is regenerated by `python sim/simulate.py`, and
`scripts/verify.sh` fails the build if a fresh run disagrees with what is
committed here.

What the runs settled:

- **2% a day** is the smallest rate that opens a visible gap inside one 60-day
  run. At 0.5%/day the two groups are still within a few percent of each other
  after two months and the mechanic is pointless; at 5%/day missing a single
  epoch costs a third of your balance, which is a punishment, not a game.
- **0.5% to stamp** is the most you can charge and still leave stamping clearly
  profitable in the half-awake case, which is the realistic one.
- **7-day stamp against 7-day epochs** is what makes the shield and the earning
  window line up.

The ghost-town run is the one worth reading twice: with nobody stamping,
nothing breaks and nobody is owed anything. The rot simply burns and supply
falls. **$ROT has no state in which it needs rescuing.**

Full write-up and charts: [`sim/RESULTS.md`](sim/RESULTS.md).

---

## Repo

```
src/math.rs        the entire ruleset — pure arithmetic, no state, no clock
src/lib.rs         the contract: state, the five verbs, NEP-141/145/148, views
sim/simulate.py    five 60-day scenarios -> sim/RESULTS.md, charts, vectors
sim/vectors.json   shared test vectors, asserted by Rust, Python and the browser
web/               the token page: live state, event log, fast-forward playground
scripts/           deploy, demo, lock
```

**[`src/math.rs`](src/math.rs) is the file to read.** It is about 150 lines and
it is the whole token. Everything else is plumbing: `lib.rs` stores numbers and
decides who may call what, the simulation replays the same functions, and the
browser mirrors them in BigInt.

The 256-bit `mul_div` in there is not decoration — balances carry 18 decimals,
so a supply-sized number is ~1e27 and the naive `a * b / d` overflows a `u128`
long before anything interesting happens.

---

## Threat model

Written before the code, kept honest afterwards. Each of these has a test.

| attack | what stops it |
|---|---|
| **Stamp a big balance, move it away, still collect** | `claim()` pays on `min(stake, balance_at_claim)`. Empty the account and you earn nothing. `stamping_then_moving_the_tokens_out_collects_nothing` |
| **Wait for a fat compost heap, then buy in** | Weight for an epoch is registered one epoch ahead, so it is sealed before any compost lands. `weight_for_a_running_epoch_cannot_be_bought_into` |
| **Split one balance across many wallets** | Rot *and* rewards are both strictly proportional to balance, so N wallets with X/N behave identically to one wallet with X. `splitting_one_stake_across_wallets_changes_nothing` |
| **Stamp twice in an epoch to double your weight** | A second stamp in the same epoch tops the existing stake up to the current balance instead of adding a new one. `stamping_twice_in_one_epoch_does_not_count_twice` |
| **Claim the same epoch repeatedly** | Each stake records what it has already collected. `claiming_twice_pays_once` |
| **Spend a balance that has already rotted** | Transfers settle both sides first. `you_cannot_send_a_balance_that_already_rotted_away` |
| **Deployer changes the rules later** | No owner, no setter, no upgrade method; `scripts/lock.ps1` removes every access key. |
| **Rot creates or destroys tokens** | `apply_rot` returns `(after, rotted)` that always sum to the input, and `audit_supply()` re-checks the books on demand. Asserted after every day of a 60-day run. |

### Known limits — stated plainly

- **The faucet is sybil-farmable.** One account, one share, and accounts are
  cheap. That is a *distribution* weakness, not a mechanism one: the rot and the
  payouts are both proportional to balance, so splitting tokens across wallets
  gains you nothing (there is a test for exactly this). The faucet exists so a
  judge can try the thing without being airdropped by us; a real launch would
  distribute through meme.cooking instead. The `storage_deposit` requirement
  puts a small NEAR cost on each sybil, and nothing more.
- **Even a perfect holder rots one day per epoch.** A stamp bought in epoch N
  shields you for `stamp_days` and earns from epoch N+1. Since the shield and
  the epoch are the same length, it lapses exactly as the epoch you are earning
  from opens — so the diligent holder's re-stamp always lands one day late and
  pays one day of rot. Stamping a day early instead keeps the shield unbroken
  but registers in epoch N again, which buys no weight in N+2. You cannot have
  both. At 2% a day that is a ~2% tax per epoch against a sleeper's ~13%, and
  the compost normally covers it several times over, but it is a real cost and
  the live demo run shows it: three stamps across two epochs left the stamper
  at 98.6% of where they started while the sleeper fell to 53%. Shortening
  `stamp_days` relative to `epoch_days` would close it and open the
  front-running hole instead.
- **A heap is rarely emptied completely.** Your payout is pro rata on the
  *smaller* of your stake and the balance you hold when you collect, and a
  stamper is normally a few percent under their stake by then — a second stamp
  costs 0.5%, and the day a shield expires rots before the next one starts. So
  a few percent of each heap goes unclaimed and is eventually burned by
  `sweep()`. This is the price of the cap, and the cap is not optional: without
  it you could stamp, move the tokens to a second account, stamp again, and have
  the same tokens counted twice in one epoch's weight, diluting everybody else.
  A leak into the burn is the cheaper of the two problems. There is a test
  (`the_sleeper_pays_the_stamper`) that pins the exact arithmetic.
- **Rot needs a poke to move.** Your *balance* falls with the clock regardless,
  but the rotted tokens only reach the compost heap when someone settles the
  account. If nobody pokes an account all epoch, that rot lands in whichever
  epoch it is finally poked in. This is deliberate — it is what makes poking
  profitable and the economy self-driving — but it means the compost in an
  epoch reflects when accounts were settled, not strictly when they rotted.
- **An account keeps only its last 16 stakes.** Stamp 17 epochs without ever
  claiming and the oldest stake is dropped; its unclaimed share stays in the
  heap and is eventually burned by `sweep()`. Bounded storage, at the cost of
  punishing extreme neglect.
- **`poke` settles at most 50 accounts per call** to stay inside the gas limit.
- **The page's clock is your computer's clock.** The chain is polled every six
  seconds; between polls the page projects balances locally. A badly wrong
  system clock shows badly wrong numbers. The contract is unaffected.
- **Not audited.** Two weeks old. Testnet only. Do not put money in it.

---

## Standards

- **NEP-141** — `ft_transfer`, `ft_transfer_call` with refund resolution,
  `ft_total_supply`, `ft_balance_of`. Implemented by hand rather than with the
  SDK's `FungibleToken` helper, because a balance that is a function of the
  clock cannot be a stored integer.
- **NEP-148** — `ft_metadata`.
- **NEP-145** — `storage_deposit`, `storage_balance_bounds`, `storage_balance_of`.
- **NEP-297** — `ft_mint` / `ft_burn` / `ft_transfer` events, plus a `rot`
  standard for `rot`, `stamp` and `claim`. The contract also keeps the last 200
  events in state so the token page can show a log of every mint and burn
  without an indexer.

## Views

`get_config`, `get_stats`, `get_account`, `list_accounts`, `get_epochs`,
`get_events`, and `audit_supply()` — which recomputes the books and returns
whether `minted − burned` still equals the faucet plus every balance plus the
unclaimed compost. Anyone can ask, any time, and the answer should never be
`false`.

---

## License

MIT — see [LICENSE](LICENSE). Built with near-sdk 5.30. Contract code written
during the hackathon window; AI-assisted, reviewed and tested by hand.
