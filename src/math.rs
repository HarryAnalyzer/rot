//! Pure mechanism math for $ROT.
//!
//! Every rule the token obeys lives in this file. No state, no clock, no I/O:
//! give it numbers, it gives you numbers. The contract calls it, the unit tests
//! hammer it, the 60-day simulation replays it, and `web/model.js` mirrors it
//! so the page can fast-forward the economy without touching a chain.
//!
//! Three rules, that is the whole coin:
//!
//!   1. ROT: an unstamped balance loses `rot_bps_per_day` of itself every day.
//!   2. STAMP: paying `stamp_cost_bps` of your balance (burned) shields you for
//!      `stamp_days` days and buys you a share of the *next* epoch's compost.
//!   3. COMPOST: everything that rots is paid out to the holders who stamped,
//!      pro rata. If nobody stamped for that epoch, it burns instead.
//!
//! Nothing here can be changed after launch. There is no admin, no setter, no
//! pause: the parameters are constructor arguments and the code above is all of
//! the policy there is.

/// Basis points denominator. 10_000 bps = 100%.
pub const BPS: u128 = 10_000;

/// Fixed-point scale for the survival factor (1e18).
pub const PRECISION: u128 = 1_000_000_000_000_000_000;

// ---------------------------------------------------------------------------
// 256-bit helpers
//
// Balances carry 18 decimals, so a whole-supply number is ~1e27 and the naive
// `a * b / d` overflows a u128 well before it gets interesting. These two do
// the multiply in 256 bits and divide back down, exactly, with no rounding
// surprises and no external dependency.
// ---------------------------------------------------------------------------

/// Full 256-bit product of two u128s, returned as `(hi, lo)`.
fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    const MASK: u128 = u64::MAX as u128;
    let (a_lo, a_hi) = (a & MASK, a >> 64);
    let (b_lo, b_hi) = (b & MASK, b >> 64);

    let ll = a_lo * b_lo;
    let lh = a_lo * b_hi;
    let hl = a_hi * b_lo;
    let hh = a_hi * b_hi;

    let mid = (ll >> 64) + (lh & MASK) + (hl & MASK);
    let lo = (ll & MASK) | (mid << 64);
    let hi = hh + (lh >> 64) + (hl >> 64) + (mid >> 64);
    (hi, lo)
}

/// `floor((hi, lo) / d)` by long division. Panics if the quotient would not fit
/// in a u128, which for this contract means a caller passed nonsense.
fn div_wide(hi: u128, lo: u128, d: u128) -> u128 {
    assert!(d > 0, "math: division by zero");
    assert!(hi < d, "math: quotient overflow");

    let mut rem = hi;
    let mut quo: u128 = 0;
    let mut i = 128;
    while i > 0 {
        i -= 1;
        let carry = rem >> 127;
        rem = (rem << 1) | ((lo >> i) & 1);
        // `carry == 1` means the shift above already lost a bit worth 2^128,
        // which is larger than any `d`, so the subtraction is owed either way.
        if carry == 1 || rem >= d {
            rem = rem.wrapping_sub(d);
            quo |= 1u128 << i;
        }
    }
    quo
}

/// Exact `floor(a * b / d)` for any u128 inputs whose quotient fits in a u128.
pub fn mul_div(a: u128, b: u128, d: u128) -> u128 {
    let (hi, lo) = mul_wide(a, b);
    div_wide(hi, lo, d)
}

// ---------------------------------------------------------------------------
// Rule 1: rot
// ---------------------------------------------------------------------------

/// How many of the days in `(settled_day, now_day]` actually rot.
///
/// `protected_until` is the first day that is *not* covered by a stamp, so a
/// stamp bought on day `d` protects `d .. d + stamp_days - 1`. Days already
/// settled never rot twice, and days still under a stamp never rot at all.
pub fn rot_days(settled_day: u64, protected_until: u64, now_day: u64) -> u64 {
    let first_rotting_day = core::cmp::max(settled_day.saturating_add(1), protected_until);
    if now_day >= first_rotting_day {
        now_day - first_rotting_day + 1
    } else {
        0
    }
}

/// `(1 - rot_bps/BPS) ^ days`, in PRECISION scale, by squaring.
///
/// Returns 0 once the factor underflows, which is the honest answer: a balance
/// left alone long enough rots away completely.
pub fn survival_factor(rot_bps_per_day: u32, days: u64) -> u128 {
    assert!((rot_bps_per_day as u128) <= BPS, "math: rot_bps > 100%");

    let mut base = PRECISION - mul_div(PRECISION, rot_bps_per_day as u128, BPS);
    let mut result = PRECISION;
    let mut n = days;

    while n > 0 {
        if n & 1 == 1 {
            result = mul_div(result, base, PRECISION);
        }
        n >>= 1;
        if n == 0 {
            break;
        }
        base = mul_div(base, base, PRECISION);
        if base == 0 {
            // Every remaining bit of the exponent multiplies by zero.
            return 0;
        }
    }
    result
}

/// Apply rot to a stored balance.
///
/// Returns `(balance_after, rotted)`. The two always sum back to `balance`:
/// rot never creates or destroys a token, it only decides who owns it.
pub fn apply_rot(balance: u128, rot_bps_per_day: u32, days: u64) -> (u128, u128) {
    if balance == 0 || days == 0 || rot_bps_per_day == 0 {
        return (balance, 0);
    }
    let factor = survival_factor(rot_bps_per_day, days);
    let after = mul_div(balance, factor, PRECISION);
    (after, balance - after)
}

// ---------------------------------------------------------------------------
// Rule 2: stamping
// ---------------------------------------------------------------------------

/// What a stamp costs right now: `stamp_cost_bps` of the settled balance.
///
/// Rounded up, so a dust balance still pays something and cannot stamp for free.
pub fn stamp_cost(balance: u128, stamp_cost_bps: u32) -> u128 {
    if balance == 0 || stamp_cost_bps == 0 {
        return 0;
    }
    let cost = mul_div(balance, stamp_cost_bps as u128, BPS);
    let cost = if cost == 0 { 1 } else { cost };
    core::cmp::min(cost, balance)
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// Day index since genesis. Before genesis everything reads as day 0.
pub fn day_of(now_ms: u64, genesis_ms: u64, day_ms: u64) -> u64 {
    assert!(day_ms > 0, "math: day_ms == 0");
    if now_ms <= genesis_ms {
        0
    } else {
        (now_ms - genesis_ms) / day_ms
    }
}

/// Epoch index that a given day belongs to.
pub fn epoch_of(day: u64, epoch_days: u64) -> u64 {
    assert!(epoch_days > 0, "math: epoch_days == 0");
    day / epoch_days
}

/// First day of an epoch.
pub fn epoch_start_day(epoch: u64, epoch_days: u64) -> u64 {
    epoch * epoch_days
}

// ---------------------------------------------------------------------------
// Rule 3: compost payout
// ---------------------------------------------------------------------------

/// A holder's cut of one finished epoch's compost.
///
/// `weight` is the total stake registered for that epoch, fixed before the
/// epoch began. `stake` is capped by the holder's balance *at claim time*, so
/// stamping a large balance and then moving it away earns nothing: the tokens
/// have to still be there when you come to collect.
///
/// The effective stake is also capped by `weight` itself. One holder cannot be
/// more than all of them, and without that cap a caller could hand in numbers
/// whose product does not fit in 256 bits.
pub fn compost_share(compost: u128, stake: u128, balance_now: u128, weight: u128) -> u128 {
    if compost == 0 || weight == 0 {
        return 0;
    }
    let effective = core::cmp::min(core::cmp::min(stake, balance_now), weight);
    if effective == 0 {
        return 0;
    }
    mul_div(compost, effective, weight)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: u128 = 1_000_000_000_000_000_000; // 1 ROT, 18 decimals
    const SUPPLY: u128 = 1_000_000_000 * ONE; // 1e27, the whole float

    // --- mul_div ---------------------------------------------------------

    #[test]
    fn mul_div_small() {
        assert_eq!(mul_div(10, 3, 4), 7); // floor(30/4)
        assert_eq!(mul_div(0, 5, 7), 0);
        assert_eq!(mul_div(7, 0, 7), 0);
    }

    #[test]
    fn mul_div_survives_whole_supply_times_precision() {
        // The case a naive u128 multiply cannot do: 1e27 * 1e18 = 1e45.
        assert_eq!(mul_div(SUPPLY, PRECISION, PRECISION), SUPPLY);
        assert_eq!(mul_div(SUPPLY, PRECISION / 2, PRECISION), SUPPLY / 2);
    }

    #[test]
    fn mul_div_supply_by_supply() {
        // Compost payouts divide one supply-sized number by another.
        assert_eq!(mul_div(SUPPLY, SUPPLY, SUPPLY), SUPPLY);
        assert_eq!(mul_div(SUPPLY, 1, SUPPLY), 1);
        assert_eq!(mul_div(SUPPLY, SUPPLY / 4, SUPPLY), SUPPLY / 4);
    }

    #[test]
    fn mul_div_matches_u128_where_u128_can_cope() {
        for (a, b, d) in [
            (123u128, 456u128, 7u128),
            (999_999u128, 1_000_003u128, 13u128),
            (u64::MAX as u128, 3, 11),
        ] {
            assert_eq!(mul_div(a, b, d), a * b / d, "a={a} b={b} d={d}");
        }
    }

    #[test]
    fn mul_div_handles_max() {
        assert_eq!(mul_div(u128::MAX, 1, 1), u128::MAX);
        assert_eq!(mul_div(u128::MAX, 2, 2), u128::MAX);
    }

    #[test]
    #[should_panic(expected = "division by zero")]
    fn mul_div_rejects_zero_divisor() {
        mul_div(1, 1, 0);
    }

    #[test]
    #[should_panic(expected = "quotient overflow")]
    fn mul_div_rejects_impossible_quotient() {
        mul_div(u128::MAX, u128::MAX, 1);
    }

    // --- rot_days --------------------------------------------------------

    #[test]
    fn nothing_rots_on_the_day_you_arrive() {
        assert_eq!(rot_days(5, 0, 5), 0);
    }

    #[test]
    fn one_day_passes_one_day_rots() {
        assert_eq!(rot_days(5, 0, 6), 1);
        assert_eq!(rot_days(5, 0, 12), 7);
    }

    #[test]
    fn a_live_stamp_blocks_every_day_it_covers() {
        // Stamped on day 5 for 7 days: protected 5..11, so day 12 is the first
        // day that rots.
        assert_eq!(rot_days(5, 12, 11), 0);
        assert_eq!(rot_days(5, 12, 12), 1);
        assert_eq!(rot_days(5, 12, 14), 3);
    }

    #[test]
    fn settled_days_never_rot_twice() {
        // Settled through day 20 with a stamp that expired back on day 12.
        assert_eq!(rot_days(20, 12, 20), 0);
        assert_eq!(rot_days(20, 12, 23), 3);
    }

    #[test]
    fn clock_going_backwards_is_not_a_credit() {
        assert_eq!(rot_days(30, 0, 10), 0);
    }

    // --- survival_factor / apply_rot -------------------------------------

    #[test]
    fn zero_days_is_identity() {
        assert_eq!(survival_factor(200, 0), PRECISION);
        assert_eq!(apply_rot(SUPPLY, 200, 0), (SUPPLY, 0));
    }

    #[test]
    fn one_day_of_two_percent() {
        let (after, rotted) = apply_rot(100 * ONE, 200, 1);
        assert_eq!(after, 98 * ONE);
        assert_eq!(rotted, 2 * ONE);
    }

    #[test]
    fn rot_compounds_it_does_not_add_up() {
        // 2%/day for 7 days is 0.98^7 = 86.86%, not 100% - 14%.
        let (after, _) = apply_rot(100 * ONE, 200, 7);
        let pct = after * 10_000 / (100 * ONE);
        assert_eq!(pct, 8681, "0.98^7 should be ~86.81%, got {pct} bps");
        assert!(after > 86 * ONE && after < 87 * ONE);
    }

    #[test]
    fn squaring_agrees_with_the_slow_honest_loop() {
        for days in [0u64, 1, 2, 3, 7, 13, 30, 64, 100] {
            let mut slow = PRECISION;
            for _ in 0..days {
                slow = mul_div(slow, PRECISION - mul_div(PRECISION, 200, BPS), PRECISION);
            }
            let fast = survival_factor(200, days);
            // Squaring rounds at different points than the loop; a few wei of
            // drift over 100 compounding steps is expected and harmless.
            let drift = slow.abs_diff(fast);
            assert!(drift <= 1_000, "days={days} slow={slow} fast={fast}");
        }
    }

    #[test]
    fn rot_conserves_tokens_exactly() {
        for days in [1u64, 5, 50, 500] {
            for balance in [1u128, ONE, 7 * ONE + 3, SUPPLY] {
                let (after, rotted) = apply_rot(balance, 200, days);
                assert_eq!(after + rotted, balance, "balance={balance} days={days}");
            }
        }
    }

    #[test]
    fn long_absence_rots_everything_and_nothing_is_lost() {
        let (after, rotted) = apply_rot(SUPPLY, 200, 100_000);
        assert_eq!(after, 0);
        assert_eq!(rotted, SUPPLY, "the whole balance must land in the compost");
    }

    #[test]
    fn a_zero_rot_token_never_spoils() {
        assert_eq!(apply_rot(SUPPLY, 0, 9_999), (SUPPLY, 0));
    }

    #[test]
    fn full_rot_takes_it_all_in_one_day() {
        assert_eq!(apply_rot(SUPPLY, 10_000, 1), (0, SUPPLY));
    }

    #[test]
    fn empty_balance_rots_nothing() {
        assert_eq!(apply_rot(0, 200, 99), (0, 0));
    }

    #[test]
    #[should_panic(expected = "rot_bps > 100%")]
    fn rot_above_one_hundred_percent_is_rejected() {
        survival_factor(10_001, 1);
    }

    // --- stamp_cost ------------------------------------------------------

    #[test]
    fn stamp_costs_its_advertised_slice() {
        assert_eq!(stamp_cost(100 * ONE, 50), ONE / 2); // 0.5%
        assert_eq!(stamp_cost(SUPPLY, 50), SUPPLY / 200);
    }

    #[test]
    fn dust_cannot_stamp_for_free() {
        assert_eq!(stamp_cost(1, 50), 1);
        assert_eq!(stamp_cost(100, 50), 1);
    }

    #[test]
    fn stamp_cost_never_exceeds_the_balance() {
        assert_eq!(stamp_cost(10, 10_000), 10);
        assert_eq!(stamp_cost(0, 50), 0);
    }

    // --- time ------------------------------------------------------------

    #[test]
    fn days_count_from_genesis() {
        let genesis = 1_000_000;
        let day = 86_400_000;
        assert_eq!(day_of(genesis, genesis, day), 0);
        assert_eq!(day_of(genesis + day - 1, genesis, day), 0);
        assert_eq!(day_of(genesis + day, genesis, day), 1);
        assert_eq!(day_of(genesis + 10 * day + 5, genesis, day), 10);
    }

    #[test]
    fn before_genesis_reads_as_day_zero() {
        assert_eq!(day_of(0, 1_000, 86_400_000), 0);
    }

    #[test]
    fn epochs_bucket_days() {
        assert_eq!(epoch_of(0, 7), 0);
        assert_eq!(epoch_of(6, 7), 0);
        assert_eq!(epoch_of(7, 7), 1);
        assert_eq!(epoch_start_day(3, 7), 21);
    }

    // --- compost_share ---------------------------------------------------

    #[test]
    fn compost_splits_pro_rata() {
        let compost = 900 * ONE;
        let weight = 300 * ONE;
        let a = compost_share(compost, 100 * ONE, 100 * ONE, weight);
        let b = compost_share(compost, 200 * ONE, 200 * ONE, weight);
        assert_eq!(a, 300 * ONE);
        assert_eq!(b, 600 * ONE);
        assert_eq!(a + b, compost, "a full epoch must pay out completely");
    }

    #[test]
    fn stamping_then_moving_the_tokens_away_earns_nothing() {
        // Stamped 200, holds 0 at claim time.
        assert_eq!(compost_share(900 * ONE, 200 * ONE, 0, 300 * ONE), 0);
    }

    #[test]
    fn the_payout_tracks_what_you_still_hold() {
        // Stamped 200 but only 50 left: paid on 50, not on 200.
        let got = compost_share(900 * ONE, 200 * ONE, 50 * ONE, 300 * ONE);
        assert_eq!(got, 150 * ONE);
    }

    #[test]
    fn holding_more_than_you_stamped_does_not_help() {
        let got = compost_share(900 * ONE, 100 * ONE, 999 * ONE, 300 * ONE);
        assert_eq!(got, 300 * ONE);
    }

    #[test]
    fn a_share_can_never_exceed_the_heap() {
        let got = compost_share(10 * ONE, SUPPLY, SUPPLY, 1);
        assert_eq!(got, 10 * ONE);
    }

    #[test]
    fn no_stake_no_compost_no_payout() {
        assert_eq!(compost_share(0, ONE, ONE, ONE), 0);
        assert_eq!(compost_share(ONE, 0, ONE, ONE), 0);
        assert_eq!(compost_share(ONE, ONE, ONE, 0), 0);
    }

    #[test]
    fn splitting_one_stake_across_wallets_changes_nothing() {
        // The sybil question, in one assertion: rot and rewards are both
        // proportional to balance, so 1 wallet with 300 and 3 wallets with 100
        // collect exactly the same compost and lose exactly the same to rot.
        let compost = 1_000 * ONE;
        let weight = 1_000 * ONE;

        let whole = compost_share(compost, 300 * ONE, 300 * ONE, weight);
        let split: u128 = (0..3)
            .map(|_| compost_share(compost, 100 * ONE, 100 * ONE, weight))
            .sum();
        assert_eq!(whole, split);

        let (_, rot_whole) = apply_rot(300 * ONE, 200, 9);
        let rot_split: u128 = (0..3).map(|_| apply_rot(100 * ONE, 200, 9).1).sum();
        assert_eq!(rot_whole, rot_split);
    }
}
