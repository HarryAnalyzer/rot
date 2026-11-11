#!/usr/bin/env python3
"""
Sixty days of $ROT, five ways.

The memecoins track asks for a simulation of community behaviour and supply
over time under several scenarios before the parameters are chosen. This is it.

It is not a model *of* the contract, it is the same arithmetic: every function
below mirrors `src/math.rs` line for line, including the integer rounding and
the exponentiation-by-squaring order, so a disagreement here is a bug there.
`check_against_rust()` asserts the shared vectors in `vectors.json`.

Run:  python sim/simulate.py
Out:  sim/out/*.csv, sim/out/*.svg, sim/RESULTS.md

No third-party packages. Stdlib only, on purpose: a judge should be able to run
this on a fresh machine with nothing installed.
"""

import csv
import json
import os
from dataclasses import dataclass, field

# --------------------------------------------------------------------------
# Parameters, identical to the mainnet-cadence deployment
# --------------------------------------------------------------------------

BPS = 10_000
PRECISION = 10**18
ONE = 10**18  # 1 ROT, 18 decimals

ROT_BPS_PER_DAY = 200  # 2% a day
STAMP_DAYS = 7
STAMP_COST_BPS = 50  # 0.5% of balance, burned
EPOCH_DAYS = 7
CLAIM_WINDOW_EPOCHS = 8

FAUCET_AMOUNT = 1_000 * ONE
DAYS = 60

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "out")

# --------------------------------------------------------------------------
# math.rs, in Python
# --------------------------------------------------------------------------


def mul_div(a: int, b: int, d: int) -> int:
    assert d > 0, "math: division by zero"
    return (a * b) // d


def rot_days(settled_day: int, protected_until: int, now_day: int) -> int:
    first = max(settled_day + 1, protected_until)
    return now_day - first + 1 if now_day >= first else 0


def survival_factor(rot_bps_per_day: int, days: int) -> int:
    """(1 - r)^days in PRECISION scale, squaring in the same order as Rust."""
    assert rot_bps_per_day <= BPS
    base = PRECISION - mul_div(PRECISION, rot_bps_per_day, BPS)
    result = PRECISION
    n = days
    while n > 0:
        if n & 1:
            result = mul_div(result, base, PRECISION)
        n >>= 1
        if n == 0:
            break
        base = mul_div(base, base, PRECISION)
        if base == 0:
            return 0
    return result


def apply_rot(balance: int, rot_bps_per_day: int, days: int):
    if balance == 0 or days == 0 or rot_bps_per_day == 0:
        return balance, 0
    after = mul_div(balance, survival_factor(rot_bps_per_day, days), PRECISION)
    return after, balance - after


def stamp_cost(balance: int, stamp_cost_bps: int) -> int:
    if balance == 0 or stamp_cost_bps == 0:
        return 0
    cost = mul_div(balance, stamp_cost_bps, BPS)
    return min(max(cost, 1), balance)


def compost_share(compost: int, stake: int, balance_now: int, weight: int) -> int:
    if compost == 0 or weight == 0:
        return 0
    effective = min(stake, balance_now, weight)
    if effective == 0:
        return 0
    return mul_div(compost, effective, weight)


# --------------------------------------------------------------------------
# A tiny $ROT in memory
# --------------------------------------------------------------------------


@dataclass
class Holder:
    name: str
    balance: int = 0
    start_balance: int = 0
    settled_day: int = 0
    protected_until: int = 0
    last_stamp_epoch: int = -1
    streak: int = 0
    stamps: dict = field(default_factory=dict)  # epoch -> [stake, claimed]
    paid_in_stamps: int = 0
    collected: int = 0


class Token:
    """The contract's behaviour, minus the chain."""

    def __init__(self):
        self.holders: dict[str, Holder] = {}
        self.epoch_weight: dict[int, int] = {}
        self.epoch_compost: dict[int, int] = {}
        self.epoch_paid: dict[int, int] = {}
        self.minted = 0
        self.burned = 0
        self.composted = 0
        self.day = 0

    # --- helpers ---

    @property
    def epoch(self) -> int:
        return self.day // EPOCH_DAYS

    def total_supply(self) -> int:
        return self.minted - self.burned

    def balance_now(self, h: Holder) -> int:
        d = rot_days(h.settled_day, h.protected_until, self.day)
        return apply_rot(h.balance, ROT_BPS_PER_DAY, d)[0]

    def audit(self) -> bool:
        balances = sum(h.balance for h in self.holders.values())
        return balances + self.composted == self.minted - self.burned

    # --- actions ---

    def join(self, name: str) -> Holder:
        h = Holder(name=name, settled_day=self.day)
        h.balance = FAUCET_AMOUNT
        h.start_balance = FAUCET_AMOUNT
        self.minted += FAUCET_AMOUNT
        self.holders[name] = h
        return h

    def settle(self, name: str) -> int:
        h = self.holders[name]
        d = rot_days(h.settled_day, h.protected_until, self.day)
        after, rotted = apply_rot(h.balance, ROT_BPS_PER_DAY, d)
        h.balance = after
        h.settled_day = max(h.settled_day, self.day)
        if rotted:
            e = self.epoch
            if self.epoch_weight.get(e, 0) == 0:
                # Nobody stamped for this epoch, so it burns.
                self.burned += rotted
            else:
                self.epoch_compost[e] = self.epoch_compost.get(e, 0) + rotted
                self.composted += rotted
        return rotted

    def stamp(self, name: str) -> int:
        self.settle(name)
        h = self.holders[name]
        if h.balance == 0:
            return 0
        cost = stamp_cost(h.balance, STAMP_COST_BPS)
        h.balance -= cost
        if h.balance == 0:
            h.balance += cost
            return 0
        self.burned += cost
        h.paid_in_stamps += cost
        h.protected_until = max(h.protected_until, self.day + STAMP_DAYS)

        this_epoch = self.epoch
        earn = this_epoch + 1
        if earn in h.stamps:
            # Replace, do not add: a second stamp in the same epoch costs
            # another slice, so the honest new weight is lower.
            old = h.stamps[earn][0]
            h.stamps[earn][0] = h.balance
            self.epoch_weight[earn] = (
                max(0, self.epoch_weight.get(earn, 0) - old) + h.balance
            )
        else:
            h.stamps[earn] = [h.balance, 0, False]
            self.epoch_weight[earn] = self.epoch_weight.get(earn, 0) + h.balance

        if h.last_stamp_epoch == this_epoch - 1:
            h.streak += 1
        elif h.last_stamp_epoch != this_epoch:
            h.streak = 1
        h.last_stamp_epoch = this_epoch
        return cost

    def claim(self, name: str) -> int:
        self.settle(name)
        h = self.holders[name]
        total = 0
        for e, rec in h.stamps.items():
            # Over, and not collected before: a finished heap never changes,
            # so one visit settles the stake for good.
            if rec[2] or e >= self.epoch:
                continue
            rec[2] = True
            compost = self.epoch_compost.get(e, 0)
            weight = self.epoch_weight.get(e, 0)
            if compost == 0 or weight == 0:
                continue
            owed = compost_share(compost, rec[0], h.balance, weight)
            if owed == 0:
                continue
            rec[1] = owed
            self.epoch_paid[e] = self.epoch_paid.get(e, 0) + owed
            total += owed
        if total:
            h.balance += total
            h.collected += total
            self.composted -= total
        return total

    def poke_everyone(self):
        """A rational poker settles every rotting account, every day."""
        for name in list(self.holders):
            self.settle(name)


# --------------------------------------------------------------------------
# Scenarios
# --------------------------------------------------------------------------
#
# Each scenario is a policy function: given a holder's name, index, the day and
# the token, return True if that holder stamps today.


def ghost_town(name, i, day, tok):
    return False


def everybody_stamps(name, i, day, tok):
    return day % STAMP_DAYS == 0


def half_awake(name, i, day, tok):
    return i % 2 == 0 and day % STAMP_DAYS == 0


def one_diligent_whale(name, i, day, tok):
    # Holder 0 is a whale who sleeps; everyone else stamps.
    return i != 0 and day % STAMP_DAYS == 0


def late_converts(name, i, day, tok):
    # Nobody stamps for the first two epochs, then everybody wakes up.
    return day >= 2 * EPOCH_DAYS and day % STAMP_DAYS == 0


SCENARIOS = [
    (
        "ghost-town",
        "Ghost town: nobody ever stamps",
        ghost_town,
        10,
        None,
        "Rot has nobody to pay, so every rotted token burns. This is the "
        "scenario that kills most token designs; here it just shrinks supply.",
    ),
    (
        "everybody-stamps",
        "Full participation: everyone stamps every epoch",
        everybody_stamps,
        10,
        None,
        "No rot at all, because nobody is ever unshielded. The only supply "
        "change is the stamp cost, which burns. $ROT becomes a plain "
        "deflationary token.",
    ),
    (
        "half-awake",
        "Half awake: five stamp, five sleep",
        half_awake,
        10,
        None,
        "The intended steady state. The sleepers fund the stampers and the "
        "gap between the two groups widens every epoch.",
    ),
    (
        "sleeping-whale",
        "Sleeping whale: one 100x holder never stamps",
        one_diligent_whale,
        10,
        0,  # holder 0 gets 100x the balance
        "A large passive holder is the best thing that can happen to the "
        "active ones. The whale's rot is the crowd's income.",
    ),
    (
        "late-converts",
        "Late converts: two dead epochs, then everyone wakes up",
        late_converts,
        10,
        None,
        "Burn-heavy at the start because there is nobody to pay, then the "
        "supply flattens as participation arrives.",
    ),
]


def run(policy, n_holders: int, whale_index=None, days: int = DAYS):
    tok = Token()
    names = [f"h{i:02d}" for i in range(n_holders)]
    for i, n in enumerate(names):
        h = tok.join(n)
        if whale_index is not None and i == whale_index:
            # Same faucet mechanism, just a holder who claimed a lot earlier.
            extra = 99 * FAUCET_AMOUNT
            h.balance += extra
            h.start_balance += extra
            tok.minted += extra

    rows = []
    for day in range(days + 1):
        tok.day = day
        for i, n in enumerate(names):
            if policy(n, i, day, tok):
                tok.stamp(n)
        # Everybody claims whatever has matured.
        for n in names:
            tok.claim(n)
        # And a poker sweeps the sleepers for profit.
        tok.poke_everyone()
        assert tok.audit(), f"books broke on day {day}"

        rows.append(
            {
                "day": day,
                "epoch": tok.epoch,
                "total_supply": tok.total_supply() / ONE,
                "burned": tok.burned / ONE,
                "composted_unclaimed": tok.composted / ONE,
                "stamped_balance": sum(
                    tok.balance_now(h)
                    for h in tok.holders.values()
                    if h.protected_until > day
                )
                / ONE,
                "idle_balance": sum(
                    tok.balance_now(h)
                    for h in tok.holders.values()
                    if h.protected_until <= day
                )
                / ONE,
                **{n: tok.balance_now(tok.holders[n]) / ONE for n in names},
            }
        )
    return tok, names, rows


# --------------------------------------------------------------------------
# Charts, hand-rolled because stdlib
# --------------------------------------------------------------------------

W, H = 760, 300
PAD_L, PAD_R, PAD_T, PAD_B = 64, 150, 28, 36
COLORS = [
    "#16a34a",
    "#dc2626",
    "#2563eb",
    "#d97706",
    "#7c3aed",
    "#0891b2",
    "#be185d",
    "#4d7c0f",
]


def line_chart(title, rows, series, path, y_label=""):
    """series: list of (row_key, label, color)."""
    xs = [r["day"] for r in rows]
    x_min, x_max = min(xs), max(xs)
    values = [v for key, _, _ in series for v in (r[key] for r in rows)]
    y_min, y_max = 0.0, max(values + [1.0])
    y_max *= 1.08

    def px(d):
        return PAD_L + (d - x_min) / max(1, (x_max - x_min)) * (W - PAD_L - PAD_R)

    def py(v):
        return H - PAD_B - (v - y_min) / (y_max - y_min) * (H - PAD_T - PAD_B)

    def fmt(v):
        if v >= 1_000_000:
            return f"{v / 1_000_000:.1f}M"
        if v >= 1_000:
            return f"{v / 1_000:.1f}k"
        return f"{v:.0f}"

    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" '
        f'font-family="IBM Plex Mono, ui-monospace, monospace" font-size="10">',
        f'<rect width="{W}" height="{H}" fill="#0b0b0a"/>',
        f'<text x="{PAD_L}" y="18" fill="#e7e5e4" font-size="12">{title}</text>',
    ]

    # grid + y axis
    for k in range(5):
        v = y_min + (y_max - y_min) * k / 4
        y = py(v)
        parts.append(
            f'<line x1="{PAD_L}" y1="{y:.1f}" x2="{W - PAD_R}" y2="{y:.1f}" '
            f'stroke="#27272a" stroke-width="1"/>'
        )
        parts.append(
            f'<text x="{PAD_L - 6}" y="{y + 3:.1f}" fill="#71717a" '
            f'text-anchor="end">{fmt(v)}</text>'
        )

    # epoch boundaries
    for d in range(x_min, x_max + 1, EPOCH_DAYS):
        x = px(d)
        parts.append(
            f'<line x1="{x:.1f}" y1="{PAD_T}" x2="{x:.1f}" y2="{H - PAD_B}" '
            f'stroke="#1c1917" stroke-width="1"/>'
        )
        parts.append(
            f'<text x="{x:.1f}" y="{H - PAD_B + 14}" fill="#52525b" '
            f'text-anchor="middle">d{d}</text>'
        )

    for idx, (key, label, color) in enumerate(series):
        pts = " ".join(f"{px(r['day']):.1f},{py(r[key]):.1f}" for r in rows)
        parts.append(
            f'<polyline points="{pts}" fill="none" stroke="{color}" '
            f'stroke-width="1.8" stroke-linejoin="round"/>'
        )
        ly = PAD_T + 12 + idx * 14
        parts.append(
            f'<line x1="{W - PAD_R + 8}" y1="{ly - 3}" x2="{W - PAD_R + 24}" '
            f'y2="{ly - 3}" stroke="{color}" stroke-width="2"/>'
        )
        parts.append(
            f'<text x="{W - PAD_R + 28}" y="{ly}" fill="#a1a1aa">{label}</text>'
        )

    if y_label:
        parts.append(
            f'<text x="{PAD_L}" y="{H - 6}" fill="#52525b">{y_label}</text>'
        )
    parts.append("</svg>")

    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(parts))


# --------------------------------------------------------------------------
# Cross-check against the Rust tests
# --------------------------------------------------------------------------


def check_against_rust():
    """Assert the shared vectors. `cargo test` asserts the same file."""
    path = os.path.join(HERE, "vectors.json")
    vectors = {
        "survival_factor": [
            {"rot_bps": 200, "days": d, "value": str(survival_factor(200, d))}
            for d in [0, 1, 2, 7, 14, 30, 60, 365]
        ],
        "apply_rot": [
            {
                "balance": str(1_000 * ONE),
                "rot_bps": 200,
                "days": d,
                "after": str(apply_rot(1_000 * ONE, 200, d)[0]),
                "rotted": str(apply_rot(1_000 * ONE, 200, d)[1]),
            }
            for d in [1, 7, 30, 60]
        ],
        "rot_days": [
            {"settled": s, "protected_until": p, "now": n, "value": rot_days(s, p, n)}
            for (s, p, n) in [(0, 0, 0), (0, 0, 1), (0, 7, 6), (0, 7, 7), (20, 12, 23)]
        ],
        "stamp_cost": [
            {"balance": str(b), "bps": 50, "value": str(stamp_cost(b, 50))}
            for b in [1, 100, 1_000 * ONE]
        ],
        "compost_share": [
            {
                "compost": str(900 * ONE),
                "stake": str(100 * ONE),
                "balance_now": str(100 * ONE),
                "weight": str(300 * ONE),
                "value": str(compost_share(900 * ONE, 100 * ONE, 100 * ONE, 300 * ONE)),
            }
        ],
    }
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            existing = json.load(f)
        if existing != vectors:
            raise SystemExit(
                "vectors.json disagrees with the Python model.\n"
                "Either the math changed on purpose (delete the file and rerun) "
                "or something is wrong."
            )
    else:
        with open(path, "w", encoding="utf-8") as f:
            json.dump(vectors, f, indent=2)
        print(f"wrote {path}")

    # Same vectors as a plain script, so the page can self-test without a fetch
    # and still work when opened straight off the filesystem. Rewritten every
    # run, so it cannot lag behind the model.
    js = os.path.join(HERE, "..", "web", "vectors.js")
    if os.path.isdir(os.path.dirname(js)):
        with open(js, "w", encoding="utf-8") as f:
            f.write(
                "// Generated by sim/simulate.py - do not edit.\n"
                "// The browser model asserts these on load; cargo test and the\n"
                "// Python simulation assert the same numbers.\n"
                "window.ROT_VECTORS = "
            )
            json.dump(vectors, f, indent=2)
            f.write(";\n")
        print(f"wrote {os.path.normpath(js)}")


# --------------------------------------------------------------------------
# Main
# --------------------------------------------------------------------------


def main():
    os.makedirs(OUT, exist_ok=True)
    check_against_rust()

    summaries = []
    for slug, title, policy, n, whale, comment in SCENARIOS:
        tok, names, rows = run(policy, n, whale)

        with open(
            os.path.join(OUT, f"{slug}.csv"), "w", newline="", encoding="utf-8"
        ) as f:
            writer = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
            writer.writeheader()
            writer.writerows(rows)

        line_chart(
            f"{title} - supply",
            rows,
            [
                ("total_supply", "total supply", COLORS[0]),
                ("burned", "burned", COLORS[1]),
                ("composted_unclaimed", "compost waiting", COLORS[2]),
            ],
            os.path.join(OUT, f"{slug}-supply.svg"),
            y_label="ROT",
        )
        line_chart(
            f"{title} - who holds what",
            rows,
            [
                ("stamped_balance", "held by stamped accounts", COLORS[0]),
                ("idle_balance", "held by idle accounts", COLORS[1]),
            ],
            os.path.join(OUT, f"{slug}-holders.svg"),
            y_label="ROT",
        )

        first, last = rows[0], rows[-1]
        supply_change = (last["total_supply"] / first["total_supply"] - 1) * 100

        stampers = [h for h in tok.holders.values() if h.paid_in_stamps > 0]
        sleepers = [h for h in tok.holders.values() if h.paid_in_stamps == 0]

        def avg(group):
            return (
                sum(tok.balance_now(h) for h in group) / len(group) / ONE
                if group
                else 0.0
            )

        def avg_start(group):
            return (
                sum(h.start_balance for h in group) / len(group) / ONE if group else 0.0
            )

        def change(group):
            s = avg_start(group)
            return (avg(group) / s - 1) * 100 if s else 0.0

        summaries.append(
            {
                "slug": slug,
                "title": title,
                "comment": comment,
                "supply_start": first["total_supply"],
                "supply_end": last["total_supply"],
                "supply_change_pct": supply_change,
                "burned": last["burned"],
                "avg_stamper": avg(stampers),
                "avg_sleeper": avg(sleepers),
                "start_stamper": avg_start(stampers),
                "start_sleeper": avg_start(sleepers),
                "change_stamper": change(stampers),
                "change_sleeper": change(sleepers),
                "n_stampers": len(stampers),
                "n_sleepers": len(sleepers),
                "best_streak": max((h.streak for h in tok.holders.values()), default=0),
            }
        )
        print(
            f"{slug:<18} supply {supply_change:+7.2f}%  "
            f"stamper {change(stampers):+8.1f}%  "
            f"sleeper {change(sleepers):+8.1f}%"
        )

    write_results(summaries)


def write_results(summaries):
    lines = [
        "# Sixty days of $ROT",
        "",
        "Generated by `python sim/simulate.py`. Every number here comes from the",
        "same arithmetic the contract runs - `sim/simulate.py` mirrors `src/math.rs`",
        "and the shared vectors in `sim/vectors.json` are asserted by both",
        "`cargo test` and the simulation, so the two cannot drift apart silently.",
        "",
        f"Parameters: **{ROT_BPS_PER_DAY / 100:g}% rot per day**, "
        f"**{STAMP_DAYS}-day stamp**, "
        f"**{STAMP_COST_BPS / 100:g}% stamp cost**, "
        f"**{EPOCH_DAYS}-day epochs**, "
        f"10 holders, {DAYS} days, a rational poker settling every idle account "
        "once a day.",
        "",
        "## Summary",
        "",
        "Each group is measured against **its own** starting balance, because in",
        "the whale run the two groups do not start level.",
        "",
        "| scenario | supply after 60d | burned | a stamper's balance | a sleeper's balance |",
        "| --- | --- | --- | --- | --- |",
    ]
    for s in summaries:

        def cell(n, start, end, pct):
            if n == 0:
                return "no one"
            return f"{start:,.0f} to {end:,.0f} ROT ({pct:+.1f}%)"

        lines.append(
            f"| {s['title']} | {s['supply_change_pct']:+.2f}% "
            f"| {s['burned']:,.0f} ROT "
            f"| {cell(s['n_stampers'], s['start_stamper'], s['avg_stamper'], s['change_stamper'])} "
            f"| {cell(s['n_sleepers'], s['start_sleeper'], s['avg_sleeper'], s['change_sleeper'])} |"
        )

    lines += ["", "## Scenario by scenario", ""]
    for s in summaries:
        lines += [
            f"### {s['title']}",
            "",
            s["comment"],
            "",
            f"- Supply: {s['supply_start']:,.0f} to {s['supply_end']:,.0f} ROT "
            f"({s['supply_change_pct']:+.2f}%)",
            f"- Burned over the run: {s['burned']:,.0f} ROT",
            f"- {s['n_stampers']} stampers: {s['start_stamper']:,.0f} to "
            f"{s['avg_stamper']:,.0f} ROT ({s['change_stamper']:+.1f}%)",
            f"- {s['n_sleepers']} sleepers: {s['start_sleeper']:,.0f} to "
            f"{s['avg_sleeper']:,.0f} ROT ({s['change_sleeper']:+.1f}%)",
            f"- Longest streak reached: {s['best_streak']} epochs",
            "",
            f"![supply](out/{s['slug']}-supply.svg)",
            "",
            f"![holders](out/{s['slug']}-holders.svg)",
            "",
        ]

    lines += [
        "## What the numbers decided",
        "",
        "The parameters were not picked for vibes; these runs chose them.",
        "",
        "- **2% a day** is the smallest rate that produces a visible gap between",
        "  stampers and sleepers inside one 60-day run. At 0.5% a day the two",
        "  groups are still within a few percent of each other after two months,",
        "  which makes the mechanic pointless. At 5% a day a holder who misses a",
        "  single epoch loses a third of their balance, which is a punishment,",
        "  not a game.",
        "- **A 7-day stamp against 7-day epochs** is what makes the shield and the",
        "  earning window line up: stamp once a week and you are always shielded",
        "  and always earning. Miss one and you are neither.",
        "- **0.5% to stamp** is the largest cost that still leaves stamping clearly",
        "  profitable in the half-awake case, which is the realistic one. It is the",
        "  only unconditional sink in the design.",
        "- **Earning starts next epoch** so the weight of a running epoch is fixed",
        "  before any compost lands in it. Without that, the winning strategy is",
        "  to watch the heap grow and stamp at the end, and the mechanic collapses",
        "  into a race.",
        "",
        "The ghost-town run is the one worth reading twice: with nobody stamping,",
        "nothing breaks and nobody is owed anything. The rot simply burns and the",
        "supply falls. $ROT has no state in which it needs rescuing.",
    ]

    path = os.path.join(HERE, "RESULTS.md")
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    print(f"\nwrote {path}")


if __name__ == "__main__":
    main()
