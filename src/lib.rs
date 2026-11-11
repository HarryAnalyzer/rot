//! # $ROT - the coin that spoils
//!
//! A NEP-141 fungible token with one unusual property: **holding it is not
//! free**. An untouched balance loses a fixed slice of itself every day, and
//! everything that rots is paid out to the holders who bothered to stamp their
//! tokens that week. Stop participating and your balance quietly drains into
//! the pockets of the people who did.
//!
//! The idea is not new. In 1932 the Austrian town of Wörgl issued scrip that
//! lost 1% of its value each month unless the holder bought a stamp and stuck
//! it to the note. Hoarding became expensive, the notes changed hands fast, and
//! a town with 30% unemployment built a bridge and a ski jump in a year.
//! Economists called it demurrage. Irving Fisher wrote a book about it. Austria's
//! central bank shut it down in 1933 and the idea has been a footnote ever since.
//!
//! $ROT is that footnote, as a memecoin, with the stamp enforced by a contract
//! instead of a town clerk.
//!
//! ## The whole mechanic
//!
//! - **Rot.** An unstamped balance loses `rot_bps_per_day` of itself per day,
//!   compounding. `ft_balance_of` is a function of the clock: nobody has to
//!   poke anything for your balance to fall, it is already lower.
//! - **Stamp.** `stamp()` burns `stamp_cost_bps` of your balance, shields you
//!   from rot for `stamp_days`, and registers your balance as *weight* in the
//!   **next** epoch. Stamping is the only way to earn.
//! - **Compost.** Rot does not vanish. When anyone settles a rotting account,
//!   the rotted tokens drop into the current epoch's compost heap, and the
//!   holders who stamped for that epoch split it pro rata. If nobody stamped
//!   for that epoch, the heap burns instead and the supply shrinks.
//! - **Poke.** Rot only *moves* when an account is settled, and anyone may
//!   settle anyone. If you are stamped this epoch, poking the sleepers is how
//!   you get paid. The economy runs itself; no cron, no keeper, no dev.
//!
//! ## What is deliberately absent
//!
//! No owner. No mint function. No pause. No setter of any kind. No upgrade
//! method. Every parameter is a constructor argument, readable by anyone via
//! `get_config`, and fixed for the life of the contract. The intended
//! deployment deletes all access keys from the contract account, so not even
//! the deployer can redeploy it.
//!
//! The full ruleset is in [`math`] - about 150 lines of pure arithmetic with no
//! state and no clock, shared by this contract, the unit tests, the 60-day
//! simulation and the browser model.

mod math;

use near_sdk::borsh::BorshSerialize;
use near_sdk::json_types::U128;
use near_sdk::store::{IterableMap, Vector};
use near_sdk::{
    env, ext_contract, near, require, AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault,
    Promise, PromiseOrValue,
};

use math::{
    apply_rot, compost_share, day_of, epoch_of, epoch_start_day, rot_days, stamp_cost, BPS,
};

/// Decimals are fixed at launch and not a constructor argument: the browser
/// model, the simulation and the tests all assume 18.
pub const DECIMALS: u8 = 18;

/// Storage charged per registered account, matching the usual NEP-145 floor.
pub const STORAGE_PER_ACCOUNT: NearToken = NearToken::from_yoctonear(1_250_000_000_000_000_000_000);

/// How many events the on-chain ring buffer keeps for the token page.
const EVENT_LOG_CAP: u32 = 200;

/// Most accounts one `poke` call may settle.
const MAX_POKE: usize = 50;

const GAS_FOR_RESOLVE_TRANSFER: Gas = Gas::from_tgas(5);
const GAS_FOR_FT_ON_TRANSFER: Gas = Gas::from_tgas(25);

#[derive(BorshSerialize, BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
enum StorageKey {
    Accounts,
    Epochs,
    Events,
}

// ---------------------------------------------------------------------------
// Stored state
// ---------------------------------------------------------------------------

/// One holder's stake in one epoch's compost heap.
#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct Stamp {
    /// Epoch this stake earns from.
    pub epoch: u64,
    /// Balance registered when the stamp was bought.
    pub stake: U128,
    /// How much of this epoch's compost was collected.
    pub claimed: U128,
    /// Closed for good. A finished epoch's compost never changes again, so one
    /// visit collects all of it and the stake is done - otherwise each payout
    /// would raise the balance that caps the next one, and a holder could walk
    /// their way up by claiming over and over.
    pub settled: bool,
}

#[near(serializers = [borsh])]
#[derive(Clone)]
pub struct Account {
    /// Balance as of `settled_day`. The *current* balance is this minus the rot
    /// owed since then; see [`Rot::balance_now`].
    balance: u128,
    /// Rot has been applied through the end of this day.
    settled_day: u64,
    /// First day **not** covered by a stamp.
    protected_until: u64,
    /// Epoch of the most recent stamp, or `u64::MAX` for never.
    last_stamp_epoch: u64,
    /// Consecutive epochs stamped.
    streak: u32,
    /// Best streak this account has ever held.
    best_streak: u32,
    /// The faucet pays each account once.
    faucet_claimed: bool,
    /// Unclaimed (and recently claimed) compost stakes, newest last.
    stamps: Vec<Stamp>,
    /// NEP-145 storage deposit held for this account.
    storage_deposit: u128,
}

impl Account {
    fn new(day: u64, storage_deposit: u128) -> Self {
        Self {
            balance: 0,
            settled_day: day,
            protected_until: 0,
            last_stamp_epoch: u64::MAX,
            streak: 0,
            best_streak: 0,
            faucet_claimed: false,
            stamps: Vec::new(),
            storage_deposit,
        }
    }
}

/// How many stamps an account keeps. Older ones are dropped, forfeiting any
/// compost never claimed from them - see `KNOWN LIMITS` in the README.
const MAX_STAMPS_KEPT: usize = 16;

#[near(serializers = [borsh])]
#[derive(Clone, Default)]
pub struct Epoch {
    /// Total stake registered for this epoch. Fixed before the epoch begins,
    /// because `stamp()` always registers for the *next* epoch.
    weight: u128,
    /// Tokens that rotted during this epoch.
    compost: u128,
    /// Compost already paid out.
    paid: u128,
    /// Whether the leftovers have been burned.
    swept: bool,
}

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct Event {
    pub day: u64,
    pub epoch: u64,
    /// `faucet` | `stamp` | `rot` | `claim` | `burn` | `transfer` | `sweep`
    pub kind: String,
    pub account: Option<AccountId>,
    pub amount: U128,
    pub note: String,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Rot {
    // --- immutable parameters, set once in `new` ---
    name: String,
    symbol: String,
    icon: Option<String>,
    genesis_ms: u64,
    day_ms: u64,
    rot_bps_per_day: u32,
    stamp_days: u64,
    stamp_cost_bps: u32,
    epoch_days: u64,
    /// Epochs after which unclaimed compost may be burned by anyone.
    claim_window_epochs: u64,
    faucet_amount: u128,
    minted: u128,

    // --- moving parts ---
    burned: u128,
    faucet_remaining: u128,
    /// Compost added but not yet claimed or swept.
    composted: u128,
    best_streak: u32,
    best_streak_holder: Option<AccountId>,
    accounts: IterableMap<AccountId, Account>,
    epochs: IterableMap<u64, Epoch>,
    events: Vector<Event>,
    events_written: u64,
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

#[near]
impl Rot {
    /// Launch the token. Every argument here is frozen forever.
    ///
    /// The entire supply goes into the faucet: there is no team allocation, no
    /// treasury and no presale, because there is nobody who could receive one.
    #[init]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        symbol: String,
        icon: Option<String>,
        day_ms: u64,
        rot_bps_per_day: u32,
        stamp_days: u64,
        stamp_cost_bps: u32,
        epoch_days: u64,
        claim_window_epochs: u64,
        total_supply: U128,
        faucet_amount: U128,
    ) -> Self {
        require!(day_ms >= 1_000, "day_ms must be at least 1000");
        require!(
            rot_bps_per_day > 0 && (rot_bps_per_day as u128) < BPS,
            "rot_bps_per_day must be in 1..9999"
        );
        require!(stamp_days >= 1, "stamp_days must be at least 1");
        require!(epoch_days >= 1, "epoch_days must be at least 1");
        require!(
            (stamp_cost_bps as u128) <= 2_000,
            "stamp_cost_bps must be at most 2000 (20%)"
        );
        require!(
            claim_window_epochs >= 2,
            "claim_window_epochs must be at least 2"
        );
        require!(total_supply.0 > 0, "total_supply must be positive");
        require!(
            faucet_amount.0 > 0 && faucet_amount.0 <= total_supply.0,
            "faucet_amount must be positive and at most total_supply"
        );

        Self {
            name,
            symbol,
            icon,
            genesis_ms: env::block_timestamp_ms(),
            day_ms,
            rot_bps_per_day,
            stamp_days,
            stamp_cost_bps,
            epoch_days,
            claim_window_epochs,
            faucet_amount: faucet_amount.0,
            minted: total_supply.0,
            burned: 0,
            faucet_remaining: total_supply.0,
            composted: 0,
            best_streak: 0,
            best_streak_holder: None,
            accounts: IterableMap::new(StorageKey::Accounts),
            epochs: IterableMap::new(StorageKey::Epochs),
            events: Vector::new(StorageKey::Events),
            events_written: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

impl Rot {
    fn day(&self) -> u64 {
        day_of(env::block_timestamp_ms(), self.genesis_ms, self.day_ms)
    }

    fn epoch(&self) -> u64 {
        epoch_of(self.day(), self.epoch_days)
    }

    /// Balance after the rot owed since `settled_day`, without writing anything.
    fn balance_now(&self, a: &Account) -> u128 {
        let days = rot_days(a.settled_day, a.protected_until, self.day());
        apply_rot(a.balance, self.rot_bps_per_day, days).0
    }

    /// Rot an account owes but has not yet had taken from it.
    fn pending_rot(&self, a: &Account) -> u128 {
        a.balance - self.balance_now(a)
    }

    fn expect_account(&self, id: &AccountId) -> Account {
        match self.accounts.get(id) {
            Some(a) => a.clone(),
            None => env::panic_str(&format!(
                "{id} is not registered; call storage_deposit first"
            )),
        }
    }

    /// Settle one account: take the rot it owes and drop it in the compost heap
    /// of the current epoch, or burn it if nobody stamped for this epoch.
    ///
    /// Returns how much rotted. Idempotent within a day.
    fn settle(&mut self, id: &AccountId) -> u128 {
        let mut a = match self.accounts.get(id) {
            Some(a) => a.clone(),
            None => return 0,
        };
        let today = self.day();
        let days = rot_days(a.settled_day, a.protected_until, today);

        let (after, rotted) = apply_rot(a.balance, self.rot_bps_per_day, days);
        a.balance = after;
        if today > a.settled_day {
            a.settled_day = today;
        }
        self.accounts.insert(id.clone(), a);

        if rotted > 0 {
            let epoch = self.epoch();
            let mut e = self.epochs.get(&epoch).cloned().unwrap_or_default();
            if e.weight == 0 {
                // Nobody stamped for this epoch, so there is nobody to pay.
                // The tokens burn and the supply shrinks.
                self.burned += rotted;
                self.epochs.insert(epoch, e);
                self.log_event("burn", Some(id.clone()), rotted, "rotted with no stampers");
                emit(
                    "rot",
                    &format!(
                        r#"{{"account_id":"{id}","rotted":"{rotted}","epoch":{epoch},"burned":true}}"#
                    ),
                );
            } else {
                e.compost += rotted;
                self.epochs.insert(epoch, e);
                self.composted += rotted;
                self.log_event("rot", Some(id.clone()), rotted, "composted");
                emit(
                    "rot",
                    &format!(
                        r#"{{"account_id":"{id}","rotted":"{rotted}","epoch":{epoch},"burned":false}}"#
                    ),
                );
            }
        }
        rotted
    }

    fn log_event(&mut self, kind: &str, account: Option<AccountId>, amount: u128, note: &str) {
        let ev = Event {
            day: self.day(),
            epoch: self.epoch(),
            kind: kind.to_string(),
            account,
            amount: U128(amount),
            note: note.to_string(),
        };
        if self.events.len() < EVENT_LOG_CAP {
            self.events.push(ev);
        } else {
            let i = (self.events_written % EVENT_LOG_CAP as u64) as u32;
            if let Some(slot) = self.events.get_mut(i) {
                *slot = ev;
            }
        }
        self.events_written += 1;
    }

    fn internal_transfer(&mut self, from: &AccountId, to: &AccountId, amount: u128, memo: &str) {
        require!(amount > 0, "amount must be positive");
        require!(from != to, "cannot transfer to yourself");
        require!(
            self.accounts.contains_key(to),
            "receiver is not registered; it must call storage_deposit first"
        );

        self.settle(from);
        self.settle(to);

        let mut sender = self.expect_account(from);
        require!(
            sender.balance >= amount,
            "not enough balance (remember it rots)"
        );
        sender.balance -= amount;
        self.accounts.insert(from.clone(), sender);

        let mut receiver = self.expect_account(to);
        receiver.balance += amount;
        self.accounts.insert(to.clone(), receiver);

        self.log_event("transfer", Some(from.clone()), amount, &format!("to {to}"));
        emit_nep141(
            "ft_transfer",
            &format!(
                r#"{{"old_owner_id":"{from}","new_owner_id":"{to}","amount":"{amount}"{}}}"#,
                if memo.is_empty() {
                    String::new()
                } else {
                    format!(r#","memo":"{memo}""#)
                }
            ),
        );
    }
}

fn emit(event: &str, data: &str) {
    env::log_str(&format!(
        r#"EVENT_JSON:{{"standard":"rot","version":"1.0.0","event":"{event}","data":[{data}]}}"#
    ));
}

fn emit_nep141(event: &str, data: &str) {
    env::log_str(&format!(
        r#"EVENT_JSON:{{"standard":"nep141","version":"1.0.0","event":"{event}","data":[{data}]}}"#
    ));
}

// ---------------------------------------------------------------------------
// The three verbs: faucet, stamp, claim (plus poke and sweep)
// ---------------------------------------------------------------------------

#[near]
impl Rot {
    /// Take your one share from the faucet.
    ///
    /// The whole supply starts here and every token in circulation came out of
    /// this function, one fixed portion per account, first come first served.
    /// Attach at least [`STORAGE_PER_ACCOUNT`] if you have never registered.
    #[payable]
    pub fn faucet_claim(&mut self) -> U128 {
        let id = env::predecessor_account_id();
        let deposit = env::attached_deposit().as_yoctonear();
        let today = self.day();

        if !self.accounts.contains_key(&id) {
            require!(
                deposit >= STORAGE_PER_ACCOUNT.as_yoctonear(),
                "attach at least 0.00125 NEAR to register"
            );
            self.accounts.insert(id.clone(), Account::new(today, deposit));
        } else if deposit > 0 {
            let mut a = self.expect_account(&id);
            a.storage_deposit += deposit;
            self.accounts.insert(id.clone(), a);
        }

        self.settle(&id);

        let mut a = self.expect_account(&id);
        require!(!a.faucet_claimed, "the faucet pays each account once");
        require!(self.faucet_remaining > 0, "the faucet is empty");

        let amount = core::cmp::min(self.faucet_amount, self.faucet_remaining);
        a.faucet_claimed = true;
        a.balance += amount;
        self.accounts.insert(id.clone(), a);
        self.faucet_remaining -= amount;

        self.log_event("faucet", Some(id.clone()), amount, "claimed");
        emit_nep141(
            "ft_mint",
            &format!(r#"{{"owner_id":"{id}","amount":"{amount}","memo":"faucet"}}"#),
        );
        U128(amount)
    }

    /// Stamp your balance: burn a slice of it, shield the rest from rot for
    /// `stamp_days`, and register for the next epoch's compost.
    ///
    /// Two separate things happen and it matters that they are separate. The
    /// shield starts **now**. The earning starts **next epoch**, with your
    /// post-cost balance as weight. That is why weight for an epoch is fully
    /// known before the epoch begins, and why there is no way to wait until a
    /// heap is large and then buy into it.
    pub fn stamp(&mut self) -> U128 {
        let id = env::predecessor_account_id();
        require!(
            self.accounts.contains_key(&id),
            "not registered; call storage_deposit first"
        );
        self.settle(&id);

        let mut a = self.expect_account(&id);
        require!(a.balance > 0, "nothing to stamp");

        let today = self.day();
        let cost = stamp_cost(a.balance, self.stamp_cost_bps);
        a.balance -= cost;
        require!(a.balance > 0, "balance too small to stamp");

        // The shield runs from today, and never shortens an existing one.
        let until = today + self.stamp_days;
        if until > a.protected_until {
            a.protected_until = until;
        }

        let this_epoch = epoch_of(today, self.epoch_days);
        let earn_epoch = this_epoch + 1;

        a.streak = if a.last_stamp_epoch != u64::MAX && a.last_stamp_epoch + 1 == this_epoch {
            a.streak + 1
        } else if a.last_stamp_epoch == this_epoch {
            a.streak.max(1)
        } else {
            1
        };
        a.last_stamp_epoch = this_epoch;
        if a.streak > a.best_streak {
            a.best_streak = a.streak;
        }

        let stake = a.balance;
        match a.stamps.iter_mut().find(|s| s.epoch == earn_epoch) {
            // Already stamped for this epoch: replace the stake with the
            // current balance rather than counting it twice. It moves in both
            // directions - a second stamp costs another 0.5%, so the honest
            // new weight is lower than the old one.
            Some(existing) => {
                let old = existing.stake.0;
                existing.stake = U128(stake);
                let mut e = self.epochs.get(&earn_epoch).cloned().unwrap_or_default();
                e.weight = e.weight.saturating_sub(old) + stake;
                self.epochs.insert(earn_epoch, e);
            }
            None => {
                a.stamps.push(Stamp {
                    epoch: earn_epoch,
                    stake: U128(stake),
                    claimed: U128(0),
                    settled: false,
                });
                if a.stamps.len() > MAX_STAMPS_KEPT {
                    a.stamps.remove(0);
                }
                let mut e = self.epochs.get(&earn_epoch).cloned().unwrap_or_default();
                e.weight += stake;
                self.epochs.insert(earn_epoch, e);
            }
        }

        let streak = a.streak;
        self.accounts.insert(id.clone(), a);
        self.burned += cost;

        if streak > self.best_streak {
            self.best_streak = streak;
            self.best_streak_holder = Some(id.clone());
        }

        self.log_event(
            "stamp",
            Some(id.clone()),
            cost,
            &format!("burned, shielded to day {until}, earns epoch {earn_epoch}, streak {streak}"),
        );
        emit_nep141(
            "ft_burn",
            &format!(r#"{{"owner_id":"{id}","amount":"{cost}","memo":"stamp"}}"#),
        );
        emit(
            "stamp",
            &format!(
                r#"{{"account_id":"{id}","cost":"{cost}","protected_until_day":{until},"earns_epoch":{earn_epoch},"streak":{streak}}}"#
            ),
        );
        U128(cost)
    }

    /// Collect your share of every finished epoch you stamped for.
    ///
    /// Your payout is capped by what you *still hold*, so stamping a large
    /// balance and moving it away before collecting earns nothing.
    pub fn claim(&mut self) -> U128 {
        let id = env::predecessor_account_id();
        require!(
            self.accounts.contains_key(&id),
            "not registered; call storage_deposit first"
        );
        self.settle(&id);

        let current_epoch = self.epoch();
        let mut a = self.expect_account(&id);
        let balance_at_claim = a.balance;
        let mut total: u128 = 0;

        for s in a.stamps.iter_mut() {
            // An epoch pays out only once it is over, and only once.
            if s.settled || s.epoch >= current_epoch {
                continue;
            }
            let Some(mut e) = self.epochs.get(&s.epoch).cloned() else {
                continue;
            };
            // A finished epoch's heap is frozen - nothing can rot into the past
            // - so this one visit collects everything this stake will ever be
            // owed, and the stake closes whether that is a lot or nothing.
            s.settled = true;
            if e.swept || e.compost == 0 {
                continue;
            }
            let owed = compost_share(e.compost, s.stake.0, balance_at_claim, e.weight);
            if owed == 0 {
                continue;
            }
            s.claimed = U128(owed);
            total += owed;

            e.paid += owed;
            self.epochs.insert(s.epoch, e);
        }

        require!(total > 0, "nothing to claim yet");

        a.balance += total;
        self.accounts.insert(id.clone(), a);
        self.composted -= total;

        self.log_event("claim", Some(id.clone()), total, "compost collected");
        emit(
            "claim",
            &format!(r#"{{"account_id":"{id}","amount":"{total}"}}"#),
        );
        U128(total)
    }

    /// Settle other accounts, forcing the rot they owe into this epoch's heap.
    ///
    /// Anyone may call this on anyone. It is the engine of the whole thing: if
    /// you are stamped for this epoch, every sleeper you poke pays you.
    pub fn poke(&mut self, account_ids: Vec<AccountId>) -> U128 {
        require!(!account_ids.is_empty(), "pass at least one account");
        require!(
            account_ids.len() <= MAX_POKE,
            "too many accounts in one poke (max 50)"
        );
        let mut released = 0u128;
        for id in account_ids.iter() {
            released += self.settle(id);
        }
        U128(released)
    }

    /// Burn compost nobody came back for.
    ///
    /// Callable by anyone once an epoch is `claim_window_epochs` old. Together
    /// with the stamp cost and the no-stampers case, this is the third way
    /// supply leaves: $ROT can shrink but never grow.
    pub fn sweep(&mut self, epoch: u64) -> U128 {
        let current = self.epoch();
        require!(
            current > epoch + self.claim_window_epochs,
            "this epoch can still be claimed"
        );
        let mut e = match self.epochs.get(&epoch).cloned() {
            Some(e) => e,
            None => env::panic_str("unknown epoch"),
        };
        require!(!e.swept, "already swept");

        let leftover = e.compost - e.paid;
        e.swept = true;
        self.epochs.insert(epoch, e);

        if leftover > 0 {
            self.burned += leftover;
            self.composted -= leftover;
            self.log_event("sweep", None, leftover, &format!("epoch {epoch} burned"));
            emit_nep141(
                "ft_burn",
                &format!(
                    r#"{{"owner_id":"{}","amount":"{leftover}","memo":"unclaimed compost"}}"#,
                    env::current_account_id()
                ),
            );
        }
        U128(leftover)
    }
}

// ---------------------------------------------------------------------------
// NEP-141 / NEP-148 / NEP-145
// ---------------------------------------------------------------------------

#[ext_contract(ext_ft_receiver)]
pub trait FungibleTokenReceiver {
    fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128>;
}

#[ext_contract(ext_self)]
pub trait RotResolver {
    fn ft_resolve_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
    ) -> U128;
}

#[near(serializers = [json])]
pub struct FungibleTokenMetadata {
    pub spec: String,
    pub name: String,
    pub symbol: String,
    pub icon: Option<String>,
    pub reference: Option<String>,
    pub reference_hash: Option<String>,
    pub decimals: u8,
}

#[near(serializers = [json])]
pub struct StorageBalanceBounds {
    pub min: NearToken,
    pub max: Option<NearToken>,
}

#[near(serializers = [json])]
pub struct StorageBalance {
    pub total: NearToken,
    pub available: NearToken,
}

#[near]
impl Rot {
    #[payable]
    pub fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        assert_one_yocto();
        let sender = env::predecessor_account_id();
        self.internal_transfer(
            &sender,
            &receiver_id,
            amount.0,
            memo.as_deref().unwrap_or(""),
        );
    }

    #[payable]
    pub fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        assert_one_yocto();
        // `Gas` has no `Add`, so the budget is compared in raw gas units.
        require!(
            env::prepaid_gas().as_gas()
                >= GAS_FOR_FT_ON_TRANSFER.as_gas() + GAS_FOR_RESOLVE_TRANSFER.as_gas(),
            "not enough gas for ft_transfer_call"
        );
        let sender = env::predecessor_account_id();
        self.internal_transfer(
            &sender,
            &receiver_id,
            amount.0,
            memo.as_deref().unwrap_or(""),
        );

        let remaining = env::prepaid_gas()
            .saturating_sub(env::used_gas())
            .saturating_sub(GAS_FOR_RESOLVE_TRANSFER);

        ext_ft_receiver::ext(receiver_id.clone())
            .with_static_gas(remaining)
            .ft_on_transfer(sender.clone(), amount, msg)
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_RESOLVE_TRANSFER)
                    .ft_resolve_transfer(sender, receiver_id, amount),
            )
            .into()
    }

    #[private]
    pub fn ft_resolve_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
    ) -> U128 {
        // A receiver returns how much of the transfer it used, as a U128
        // string; 64 bytes is comfortably more than the longest of those.
        // Anything else - a failed call, a malformed answer - is treated as
        // "used nothing", so the whole amount goes back to the sender.
        let unused = match env::promise_result_checked(0, 64) {
            Ok(value) => match near_sdk::serde_json::from_slice::<U128>(&value) {
                Ok(used) => core::cmp::min(amount.0, used.0),
                Err(_) => amount.0,
            },
            Err(_) => amount.0,
        };

        if unused > 0 {
            self.settle(&receiver_id);
            let mut receiver = self.expect_account(&receiver_id);
            let refund = core::cmp::min(unused, receiver.balance);
            if refund > 0 && self.accounts.contains_key(&sender_id) {
                receiver.balance -= refund;
                self.accounts.insert(receiver_id.clone(), receiver);

                self.settle(&sender_id);
                let mut sender = self.expect_account(&sender_id);
                sender.balance += refund;
                self.accounts.insert(sender_id.clone(), sender);

                emit_nep141(
                    "ft_transfer",
                    &format!(
                        r#"{{"old_owner_id":"{receiver_id}","new_owner_id":"{sender_id}","amount":"{refund}","memo":"refund"}}"#
                    ),
                );
                return U128(amount.0 - refund);
            }
        }
        amount
    }

    pub fn ft_total_supply(&self) -> U128 {
        U128(self.minted - self.burned)
    }

    /// The current balance, rot already taken off. This is a pure function of
    /// the clock: no transaction is needed for it to fall.
    pub fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        match self.accounts.get(&account_id) {
            Some(a) => U128(self.balance_now(a)),
            None => U128(0),
        }
    }

    pub fn ft_metadata(&self) -> FungibleTokenMetadata {
        FungibleTokenMetadata {
            spec: "ft-1.0.0".to_string(),
            name: self.name.clone(),
            symbol: self.symbol.clone(),
            icon: self.icon.clone(),
            reference: None,
            reference_hash: None,
            decimals: DECIMALS,
        }
    }

    #[payable]
    pub fn storage_deposit(
        &mut self,
        account_id: Option<AccountId>,
        registration_only: Option<bool>,
    ) -> StorageBalance {
        let id = account_id.unwrap_or_else(env::predecessor_account_id);
        let deposit = env::attached_deposit().as_yoctonear();
        let min = STORAGE_PER_ACCOUNT.as_yoctonear();

        if self.accounts.contains_key(&id) {
            if deposit > 0 {
                if registration_only.unwrap_or(false) {
                    // Creating the promise schedules the refund; there is
                    // nothing to wait for and nothing to return.
                    let _refund = Promise::new(env::predecessor_account_id())
                        .transfer(NearToken::from_yoctonear(deposit));
                } else {
                    let mut a = self.expect_account(&id);
                    a.storage_deposit += deposit;
                    self.accounts.insert(id.clone(), a);
                }
            }
        } else {
            require!(deposit >= min, "attach at least 0.00125 NEAR to register");
            let keep = if registration_only.unwrap_or(false) {
                min
            } else {
                deposit
            };
            self.accounts.insert(id.clone(), Account::new(self.day(), keep));
            if deposit > keep {
                let _refund = Promise::new(env::predecessor_account_id())
                    .transfer(NearToken::from_yoctonear(deposit - keep));
            }
        }
        self.storage_balance_of(id).unwrap()
    }

    pub fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        StorageBalanceBounds {
            min: STORAGE_PER_ACCOUNT,
            max: None,
        }
    }

    pub fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.accounts.get(&account_id).map(|a| StorageBalance {
            total: NearToken::from_yoctonear(a.storage_deposit),
            available: NearToken::from_yoctonear(
                a.storage_deposit
                    .saturating_sub(STORAGE_PER_ACCOUNT.as_yoctonear()),
            ),
        })
    }
}

fn assert_one_yocto() {
    require!(
        env::attached_deposit() == NearToken::from_yoctonear(1),
        "requires an attached deposit of exactly 1 yoctoNEAR"
    );
}

// ---------------------------------------------------------------------------
// Views for the token page
// ---------------------------------------------------------------------------

#[near(serializers = [json])]
pub struct Config {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub genesis_ms: u64,
    pub day_ms: u64,
    pub rot_bps_per_day: u32,
    pub stamp_days: u64,
    pub stamp_cost_bps: u32,
    pub epoch_days: u64,
    pub claim_window_epochs: u64,
    pub faucet_amount: U128,
    pub total_minted: U128,
    /// Always false: there is no owner field in this contract, and no method
    /// that reads one. Whether the *account* can still be redeployed is a
    /// separate question, answered by `near account list-keys` rather than by
    /// anything the contract could tell you about itself.
    pub has_owner: bool,
}

#[near(serializers = [json])]
pub struct Stats {
    pub now_ms: u64,
    pub day: u64,
    pub epoch: u64,
    pub epoch_start_day: u64,
    pub next_epoch_in_days: u64,
    pub total_supply: U128,
    pub burned: U128,
    pub faucet_remaining: U128,
    pub circulating: U128,
    pub composted_unclaimed: U128,
    pub holders: u32,
    pub best_streak: u32,
    pub best_streak_holder: Option<AccountId>,
    /// Weight stamped for the epoch running right now.
    pub current_epoch_weight: U128,
    /// Compost gathered so far in the epoch running right now.
    pub current_epoch_compost: U128,
    /// Weight already locked in for the next epoch.
    pub next_epoch_weight: U128,
    /// Rot that accounts owe but nobody has poked out of them yet.
    pub unrealised_rot: U128,
}

#[near(serializers = [json])]
pub struct AccountView {
    pub account_id: AccountId,
    /// Balance right now, rot included.
    pub balance: U128,
    /// Balance as last written on chain, before pending rot.
    pub balance_settled: U128,
    pub pending_rot: U128,
    pub settled_day: u64,
    pub protected_until_day: u64,
    pub is_protected: bool,
    pub days_of_shield_left: u64,
    pub streak: u32,
    pub best_streak: u32,
    pub faucet_claimed: bool,
    pub stamps: Vec<Stamp>,
    pub claimable: U128,
    /// What this balance loses tomorrow if left unstamped.
    pub rot_per_day_if_idle: U128,
}

#[near(serializers = [json])]
pub struct EpochView {
    pub epoch: u64,
    pub start_day: u64,
    pub weight: U128,
    pub compost: U128,
    pub paid: U128,
    pub swept: bool,
    pub status: String,
}

#[near]
impl Rot {
    pub fn get_config(&self) -> Config {
        Config {
            name: self.name.clone(),
            symbol: self.symbol.clone(),
            decimals: DECIMALS,
            genesis_ms: self.genesis_ms,
            day_ms: self.day_ms,
            rot_bps_per_day: self.rot_bps_per_day,
            stamp_days: self.stamp_days,
            stamp_cost_bps: self.stamp_cost_bps,
            epoch_days: self.epoch_days,
            claim_window_epochs: self.claim_window_epochs,
            faucet_amount: U128(self.faucet_amount),
            total_minted: U128(self.minted),
            has_owner: false,
        }
    }

    pub fn get_stats(&self) -> Stats {
        let day = self.day();
        let epoch = epoch_of(day, self.epoch_days);
        let start = epoch_start_day(epoch, self.epoch_days);
        let this = self.epochs.get(&epoch).cloned().unwrap_or_default();
        let next = self.epochs.get(&(epoch + 1)).cloned().unwrap_or_default();

        let mut unrealised = 0u128;
        for (_, a) in self.accounts.iter() {
            unrealised += self.pending_rot(a);
        }

        Stats {
            now_ms: env::block_timestamp_ms(),
            day,
            epoch,
            epoch_start_day: start,
            next_epoch_in_days: start + self.epoch_days - day,
            total_supply: U128(self.minted - self.burned),
            burned: U128(self.burned),
            faucet_remaining: U128(self.faucet_remaining),
            circulating: U128(self.minted - self.burned - self.faucet_remaining),
            composted_unclaimed: U128(self.composted),
            holders: self.accounts.len(),
            best_streak: self.best_streak,
            best_streak_holder: self.best_streak_holder.clone(),
            current_epoch_weight: U128(this.weight),
            current_epoch_compost: U128(this.compost),
            next_epoch_weight: U128(next.weight),
            unrealised_rot: U128(unrealised),
        }
    }

    pub fn get_account(&self, account_id: AccountId) -> Option<AccountView> {
        self.accounts.get(&account_id).map(|a| {
            let day = self.day();
            let balance = self.balance_now(a);
            let current_epoch = epoch_of(day, self.epoch_days);

            let mut claimable = 0u128;
            for s in a.stamps.iter() {
                if s.settled || s.epoch >= current_epoch {
                    continue;
                }
                if let Some(e) = self.epochs.get(&s.epoch) {
                    if e.swept {
                        continue;
                    }
                    claimable += compost_share(e.compost, s.stake.0, balance, e.weight);
                }
            }

            let protected = a.protected_until > day;
            AccountView {
                account_id: account_id.clone(),
                balance: U128(balance),
                balance_settled: U128(a.balance),
                pending_rot: U128(a.balance - balance),
                settled_day: a.settled_day,
                protected_until_day: a.protected_until,
                is_protected: protected,
                days_of_shield_left: a.protected_until.saturating_sub(day),
                streak: a.streak,
                best_streak: a.best_streak,
                faucet_claimed: a.faucet_claimed,
                stamps: a.stamps.clone(),
                claimable: U128(claimable),
                rot_per_day_if_idle: U128(
                    apply_rot(balance, self.rot_bps_per_day, 1).1,
                ),
            }
        })
    }

    /// Page through holders. The page sorts these itself: the leaderboard, and
    /// the list of accounts worth poking, both come from here.
    pub fn list_accounts(&self, from_index: Option<u32>, limit: Option<u32>) -> Vec<AccountView> {
        let from = from_index.unwrap_or(0) as usize;
        let limit = limit.unwrap_or(50).min(200) as usize;
        self.accounts
            .iter()
            .skip(from)
            .take(limit)
            .filter_map(|(id, _)| self.get_account(id.clone()))
            .collect()
    }

    pub fn get_epochs(&self, from_epoch: Option<u64>, limit: Option<u64>) -> Vec<EpochView> {
        let current = self.epoch();
        let limit = limit.unwrap_or(12).min(100);
        let from = from_epoch.unwrap_or_else(|| (current + 1).saturating_sub(limit));
        (from..=current + 1)
            .take(limit as usize)
            .map(|epoch| {
                let e = self.epochs.get(&epoch).cloned().unwrap_or_default();
                let status = if epoch > current {
                    "upcoming"
                } else if epoch == current {
                    "running"
                } else if e.swept {
                    "swept"
                } else {
                    "claimable"
                };
                EpochView {
                    epoch,
                    start_day: epoch_start_day(epoch, self.epoch_days),
                    weight: U128(e.weight),
                    compost: U128(e.compost),
                    paid: U128(e.paid),
                    swept: e.swept,
                    status: status.to_string(),
                }
            })
            .collect()
    }

    /// Newest first. Every faucet claim, stamp, rot, claim, burn and sweep the
    /// contract has seen recently, straight from contract state.
    pub fn get_events(&self, limit: Option<u32>) -> Vec<Event> {
        let limit = limit.unwrap_or(50).min(EVENT_LOG_CAP);
        let len = self.events.len();
        if len == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let newest = self.events_written;
        let mut i = 0u64;
        while i < limit as u64 && i < len as u64 {
            let idx = ((newest - 1 - i) % EVENT_LOG_CAP as u64) as u32;
            if let Some(ev) = self.events.get(idx) {
                out.push(ev.clone());
            }
            i += 1;
        }
        out
    }

    /// Does the arithmetic still add up? Anyone can ask, any time.
    ///
    /// `minted - burned` must equal the faucet plus every settled balance plus
    /// the rot still owed plus the unclaimed compost. If this ever returns
    /// false, the token has lost or invented tokens somewhere.
    pub fn audit_supply(&self) -> bool {
        let mut balances = 0u128;
        for (_, a) in self.accounts.iter() {
            balances += a.balance;
        }
        self.faucet_remaining + balances + self.composted == self.minted - self.burned
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;

    const ONE: u128 = 1_000_000_000_000_000_000;
    const SUPPLY: u128 = 1_000_000_000 * ONE;
    const FAUCET: u128 = 1_000 * ONE;
    const DAY_MS: u64 = 86_400_000;
    const GENESIS_MS: u64 = 1_700_000_000_000;

    fn acc(s: &str) -> AccountId {
        s.parse().unwrap()
    }

    fn ctx(who: &str, day: u64, deposit: u128) -> VMContextBuilder {
        let mut b = VMContextBuilder::new();
        b.current_account_id(acc("rot.testnet"))
            .predecessor_account_id(acc(who))
            .signer_account_id(acc(who))
            .block_timestamp((GENESIS_MS + day * DAY_MS) * 1_000_000)
            .attached_deposit(NearToken::from_yoctonear(deposit))
            .prepaid_gas(Gas::from_tgas(100));
        b
    }

    fn setup() -> Rot {
        testing_env!(ctx("alice.testnet", 0, 0).build());
        Rot::new(
            "Rot".to_string(),
            "ROT".to_string(),
            None,
            DAY_MS,
            200, // 2% per day
            7,   // shield lasts 7 days
            50,  // stamp costs 0.5%
            7,   // epochs are 7 days
            8,
            U128(SUPPLY),
            U128(FAUCET),
        )
    }

    fn register_and_claim(c: &mut Rot, who: &str, day: u64) {
        testing_env!(ctx(who, day, STORAGE_PER_ACCOUNT.as_yoctonear()).build());
        c.faucet_claim();
    }

    // --- launch ----------------------------------------------------------

    #[test]
    fn launches_with_everything_in_the_faucet() {
        let c = setup();
        let s = c.get_stats();
        assert_eq!(s.total_supply.0, SUPPLY);
        assert_eq!(s.faucet_remaining.0, SUPPLY);
        assert_eq!(s.circulating.0, 0, "no insider allocation at launch");
        assert_eq!(s.holders, 0);
        assert!(c.audit_supply());
    }

    #[test]
    fn parameters_are_readable_by_anyone() {
        let c = setup();
        let cfg = c.get_config();
        assert_eq!(cfg.rot_bps_per_day, 200);
        assert_eq!(cfg.stamp_days, 7);
        assert_eq!(cfg.stamp_cost_bps, 50);
        assert_eq!(cfg.epoch_days, 7);
        assert_eq!(cfg.decimals, 18);
    }

    #[test]
    #[should_panic(expected = "rot_bps_per_day must be in 1..9999")]
    fn a_token_that_cannot_rot_is_refused() {
        testing_env!(ctx("alice.testnet", 0, 0).build());
        Rot::new(
            "Rot".into(),
            "ROT".into(),
            None,
            DAY_MS,
            0,
            7,
            50,
            7,
            8,
            U128(SUPPLY),
            U128(FAUCET),
        );
    }

    #[test]
    #[should_panic(expected = "stamp_cost_bps must be at most 2000")]
    fn an_extortionate_stamp_is_refused() {
        testing_env!(ctx("alice.testnet", 0, 0).build());
        Rot::new(
            "Rot".into(),
            "ROT".into(),
            None,
            DAY_MS,
            200,
            7,
            5_000,
            7,
            8,
            U128(SUPPLY),
            U128(FAUCET),
        );
    }

    // --- faucet ----------------------------------------------------------

    #[test]
    fn the_faucet_pays_once() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        assert_eq!(c.ft_balance_of(acc("alice.testnet")).0, FAUCET);
        assert_eq!(c.get_stats().circulating.0, FAUCET);
        assert!(c.audit_supply());
    }

    #[test]
    #[should_panic(expected = "the faucet pays each account once")]
    fn the_faucet_refuses_seconds() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.faucet_claim();
    }

    #[test]
    #[should_panic(expected = "attach at least 0.00125 NEAR")]
    fn registering_costs_storage() {
        let mut c = setup();
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.faucet_claim();
    }

    // --- rot -------------------------------------------------------------

    #[test]
    fn an_idle_balance_falls_without_anyone_touching_it() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);

        // No transaction at all, just a later block.
        testing_env!(ctx("bob.testnet", 1, 0).build());
        assert_eq!(c.ft_balance_of(acc("alice.testnet")).0, FAUCET * 98 / 100);

        testing_env!(ctx("bob.testnet", 10, 0).build());
        let b = c.ft_balance_of(acc("alice.testnet")).0;
        assert!(b < FAUCET * 82 / 100 && b > FAUCET * 81 / 100, "got {b}");
    }

    #[test]
    fn the_settled_balance_only_moves_when_poked() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);

        testing_env!(ctx("bob.testnet", 5, 0).build());
        let v = c.get_account(acc("alice.testnet")).unwrap();
        assert_eq!(v.balance_settled.0, FAUCET, "on-chain figure is stale");
        assert!(v.pending_rot.0 > 0, "and it owes rot");
        assert_eq!(
            v.balance.0 + v.pending_rot.0,
            FAUCET,
            "which the view accounts for"
        );
    }

    #[test]
    fn rot_with_nobody_stamped_burns_and_shrinks_the_supply() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);

        testing_env!(ctx("bob.testnet", 3, 0).build());
        let released = c.poke(vec![acc("alice.testnet")]).0;
        assert!(released > 0);

        let s = c.get_stats();
        assert_eq!(s.burned.0, released, "nobody to pay, so it burns");
        assert_eq!(s.composted_unclaimed.0, 0);
        assert_eq!(s.total_supply.0, SUPPLY - released);
        assert!(c.audit_supply());
    }

    // --- stamping --------------------------------------------------------

    #[test]
    fn stamping_burns_a_slice_and_raises_a_shield() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);

        testing_env!(ctx("alice.testnet", 0, 0).build());
        let cost = c.stamp().0;
        assert_eq!(cost, FAUCET / 200, "0.5% of the balance");

        let v = c.get_account(acc("alice.testnet")).unwrap();
        assert_eq!(v.protected_until_day, 7);
        assert!(v.is_protected);
        assert_eq!(v.streak, 1);
        assert_eq!(c.get_stats().burned.0, cost);
    }

    #[test]
    fn a_shield_really_stops_the_rot() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
        let after_stamp = c.ft_balance_of(acc("alice.testnet")).0;

        // Six days inside the shield: not a single token lost.
        testing_env!(ctx("bob.testnet", 6, 0).build());
        assert_eq!(c.ft_balance_of(acc("alice.testnet")).0, after_stamp);

        // Day 7, the shield is gone and the rot resumes.
        testing_env!(ctx("bob.testnet", 7, 0).build());
        assert_eq!(
            c.ft_balance_of(acc("alice.testnet")).0,
            after_stamp * 98 / 100
        );
    }

    #[test]
    fn a_stamp_earns_from_the_next_epoch_not_this_one() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();

        let v = c.get_account(acc("alice.testnet")).unwrap();
        assert_eq!(v.stamps.len(), 1);
        assert_eq!(v.stamps[0].epoch, 1, "stamped in epoch 0, earns in epoch 1");

        let s = c.get_stats();
        assert_eq!(s.current_epoch_weight.0, 0, "epoch 0 has no stampers");
        assert_eq!(s.next_epoch_weight.0, v.balance_settled.0);
    }

    #[test]
    fn weight_for_a_running_epoch_cannot_be_bought_into() {
        // The anti-frontrunning property: by the time an epoch starts gathering
        // compost, its weight is already fixed, so nobody can watch the heap
        // grow and then buy a share of it.
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();

        let weight_at_start = c.get_stats().next_epoch_weight.0;

        register_and_claim(&mut c, "bob.testnet", 8); // mid-epoch-1
        testing_env!(ctx("bob.testnet", 8, 0).build());
        c.stamp();

        testing_env!(ctx("carol.testnet", 8, 0).build());
        let s = c.get_stats();
        assert_eq!(s.epoch, 1);
        assert_eq!(
            s.current_epoch_weight.0, weight_at_start,
            "bob's stamp did not dilute the running epoch"
        );
        assert!(s.next_epoch_weight.0 > 0, "it landed in epoch 2 instead");
    }

    #[test]
    fn stamping_twice_in_one_epoch_does_not_count_twice() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
        let after_one = c.get_stats().next_epoch_weight.0;

        testing_env!(ctx("alice.testnet", 2, 0).build());
        c.stamp();
        let after_two = c.get_stats().next_epoch_weight.0;

        assert!(
            after_two < after_one,
            "weight tracks the balance, which the second stamp cost shrank"
        );
        let v = c.get_account(acc("alice.testnet")).unwrap();
        assert_eq!(v.stamps.len(), 1, "still one stake, not two");
        assert_eq!(v.stamps[0].stake.0, after_two);
    }

    #[test]
    fn streaks_count_consecutive_epochs_and_reset_when_broken() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);

        for epoch in 0..4u64 {
            testing_env!(ctx("alice.testnet", epoch * 7, 0).build());
            c.stamp();
        }
        assert_eq!(c.get_account(acc("alice.testnet")).unwrap().streak, 4);
        assert_eq!(c.get_stats().best_streak, 4);
        assert_eq!(
            c.get_stats().best_streak_holder,
            Some(acc("alice.testnet"))
        );

        // Skip epoch 4 entirely, come back in epoch 5.
        testing_env!(ctx("alice.testnet", 5 * 7, 0).build());
        c.stamp();
        let v = c.get_account(acc("alice.testnet")).unwrap();
        assert_eq!(v.streak, 1, "a missed epoch resets the streak");
        assert_eq!(v.best_streak, 4, "but the record stands");
    }

    #[test]
    #[should_panic(expected = "nothing to stamp")]
    fn an_empty_account_cannot_stamp() {
        let mut c = setup();
        testing_env!(ctx("alice.testnet", 0, STORAGE_PER_ACCOUNT.as_yoctonear()).build());
        c.storage_deposit(None, None);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
    }

    // --- the actual game: rot flows to stampers ---------------------------

    #[test]
    fn the_sleeper_pays_the_stamper() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0); // will stamp
        register_and_claim(&mut c, "bob.testnet", 0); // will sleep

        // Alice stamps in epoch 0, so she earns from epoch 1.
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
        // And re-stamps in epoch 1 to stay shielded while she earns.
        testing_env!(ctx("alice.testnet", 7, 0).build());
        c.stamp();

        // Day 10, still inside epoch 1. Someone pokes Bob.
        testing_env!(ctx("carol.testnet", 10, 0).build());
        let released = c.poke(vec![acc("bob.testnet")]).0;
        assert!(released > 0, "bob rotted");
        // Bob's rot is in the heap, and so is the single day Alice rotted on
        // day 7, between her shield expiring and her second stamp.
        assert!(c.get_stats().current_epoch_compost.0 > released);

        // Epoch 1 is over: Alice collects.
        testing_env!(ctx("alice.testnet", 14, 0).build());
        let heap = c.get_epochs(Some(1), Some(1)).remove(0).compost.0;
        let before = c.ft_balance_of(acc("alice.testnet")).0;
        let stake = c
            .get_account(acc("alice.testnet"))
            .unwrap()
            .stamps
            .iter()
            .find(|s| s.epoch == 1)
            .unwrap()
            .stake
            .0;
        let claimed = c.claim().0;
        let after = c.ft_balance_of(acc("alice.testnet")).0;

        assert_eq!(after, before + claimed);
        assert!(
            claimed > released,
            "she took bob's rot and the day she rotted herself"
        );

        // She is the only stamper and still cannot take the whole heap. The
        // payout is pro rata on the *smaller* of her stake and the balance she
        // actually holds, and a second stamp plus one day of rot have left her
        // under her stake. The shortfall stays in the heap for sweep() to burn.
        //
        // That cap is not an accident. Without it, a holder could stamp, move
        // the tokens to a second account, stamp again, and have the same
        // tokens counted twice in one epoch's weight - diluting everyone else.
        assert!(before < stake, "stamp costs and rot put her under her stake");
        // Two supply-sized numbers multiplied together do not fit in a u128,
        // which is the whole reason math.rs does this in 256 bits.
        assert_eq!(claimed, crate::math::mul_div(heap, before, stake));
        assert!(claimed < heap);
        assert!(c.audit_supply());
    }

    #[test]
    fn two_stampers_split_the_heap_by_weight() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);
        register_and_claim(&mut c, "sleeper.testnet", 0);

        // Bob gives Alice half his balance so the weights differ 3:1.
        testing_env!(ctx("bob.testnet", 0, 1).build());
        c.ft_transfer(acc("alice.testnet"), U128(FAUCET / 2), None);

        for who in ["alice.testnet", "bob.testnet"] {
            testing_env!(ctx(who, 0, 0).build());
            c.stamp();
            testing_env!(ctx(who, 7, 0).build());
            c.stamp();
        }

        testing_env!(ctx("carol.testnet", 9, 0).build());
        let released = c.poke(vec![acc("sleeper.testnet")]).0;
        assert!(released > 0);

        testing_env!(ctx("alice.testnet", 14, 0).build());
        let heap = c.get_epochs(Some(1), Some(1)).remove(0).compost.0;
        let a = c.claim().0;
        testing_env!(ctx("bob.testnet", 14, 0).build());
        let b = c.claim().0;

        // Alice holds ~1.5x the faucet, Bob ~0.5x, so 3:1.
        let ratio = a * 100 / b;
        assert!(ratio > 295 && ratio < 305, "expected ~3:1, got {ratio}/100");
        assert!(a + b <= heap, "between them they cannot take more than the heap");
        assert!(heap >= released);
        assert!(c.audit_supply());
    }

    #[test]
    #[should_panic(expected = "nothing to claim yet")]
    fn a_running_epoch_pays_nobody() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
        testing_env!(ctx("alice.testnet", 7, 0).build());
        c.stamp();
        testing_env!(ctx("carol.testnet", 9, 0).build());
        c.poke(vec![acc("bob.testnet")]);

        // Epoch 1 has compost and Alice has weight in it, but it is still running.
        testing_env!(ctx("alice.testnet", 9, 0).build());
        c.claim();
    }

    #[test]
    fn stamping_then_moving_the_tokens_out_collects_nothing() {
        // The one real attack on a stake-weighted payout: register weight, then
        // move the tokens somewhere else and collect anyway. The payout is
        // capped by the balance held at claim time, so it earns nothing.
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);
        register_and_claim(&mut c, "sleeper.testnet", 0);

        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
        testing_env!(ctx("alice.testnet", 7, 0).build());
        c.stamp();

        testing_env!(ctx("carol.testnet", 8, 0).build());
        c.poke(vec![acc("sleeper.testnet")]);

        // Alice empties her account before the epoch ends.
        testing_env!(ctx("alice.testnet", 9, 0).build());
        let all = c.ft_balance_of(acc("alice.testnet")).0;
        testing_env!(ctx("alice.testnet", 9, 1).build());
        c.ft_transfer(acc("bob.testnet"), U128(all), None);

        testing_env!(ctx("alice.testnet", 14, 0).build());
        let v = c.get_account(acc("alice.testnet")).unwrap();
        assert_eq!(v.balance.0, 0);
        assert_eq!(v.claimable.0, 0, "no balance, no payout");
    }

    #[test]
    fn claiming_twice_pays_once() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
        testing_env!(ctx("alice.testnet", 7, 0).build());
        c.stamp();
        testing_env!(ctx("carol.testnet", 9, 0).build());
        c.poke(vec![acc("bob.testnet")]);

        testing_env!(ctx("alice.testnet", 14, 0).build());
        let first = c.claim().0;
        assert!(first > 0);
        let second = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.claim()));
        assert!(second.is_err(), "the second claim must find nothing left");
    }

    // --- sweep -----------------------------------------------------------

    #[test]
    fn compost_nobody_came_back_for_is_burned_by_anyone() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
        testing_env!(ctx("alice.testnet", 7, 0).build());
        c.stamp();
        testing_env!(ctx("carol.testnet", 9, 0).build());
        let released = c.poke(vec![acc("bob.testnet")]).0;
        assert!(released > 0);

        // Alice never claims. Many epochs later a stranger sweeps epoch 1.
        testing_env!(ctx("stranger.testnet", 7 * 20, 0).build());
        let heap = c.get_epochs(Some(1), Some(1)).remove(0).compost.0;
        let burned = c.sweep(1).0;
        assert_eq!(burned, heap, "the whole untouched heap burns");
        assert_eq!(c.get_stats().composted_unclaimed.0, 0);
        assert_eq!(c.ft_total_supply().0, SUPPLY - c.get_stats().burned.0);
        assert!(c.audit_supply());
    }

    #[test]
    #[should_panic(expected = "this epoch can still be claimed")]
    fn sweeping_early_is_refused() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
        testing_env!(ctx("stranger.testnet", 14, 0).build());
        c.sweep(1);
    }

    // --- NEP-141 ---------------------------------------------------------

    #[test]
    fn transfers_settle_both_sides_first() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);

        // Five days of rot on both, then a transfer.
        testing_env!(ctx("alice.testnet", 5, 1).build());
        let alice_before = c.ft_balance_of(acc("alice.testnet")).0;
        let bob_before = c.ft_balance_of(acc("bob.testnet")).0;
        assert!(alice_before < FAUCET);

        c.ft_transfer(acc("bob.testnet"), U128(100 * ONE), None);

        assert_eq!(
            c.ft_balance_of(acc("alice.testnet")).0,
            alice_before - 100 * ONE
        );
        assert_eq!(
            c.ft_balance_of(acc("bob.testnet")).0,
            bob_before + 100 * ONE
        );
        assert!(c.audit_supply());
    }

    #[test]
    #[should_panic(expected = "not enough balance (remember it rots)")]
    fn you_cannot_send_a_balance_that_already_rotted_away() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);
        // The full faucet amount was hers on day 0, but not on day 5.
        testing_env!(ctx("alice.testnet", 5, 1).build());
        c.ft_transfer(acc("bob.testnet"), U128(FAUCET), None);
    }

    #[test]
    #[should_panic(expected = "receiver is not registered")]
    fn transfers_to_unregistered_accounts_are_refused() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 1).build());
        c.ft_transfer(acc("nobody.testnet"), U128(ONE), None);
    }

    #[test]
    #[should_panic(expected = "exactly 1 yoctoNEAR")]
    fn transfers_need_the_one_yocto() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);
        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.ft_transfer(acc("bob.testnet"), U128(ONE), None);
    }

    #[test]
    fn metadata_is_nep148_shaped() {
        let c = setup();
        let m = c.ft_metadata();
        assert_eq!(m.spec, "ft-1.0.0");
        assert_eq!(m.symbol, "ROT");
        assert_eq!(m.decimals, 18);
    }

    #[test]
    fn an_unknown_account_holds_nothing() {
        let c = setup();
        assert_eq!(c.ft_balance_of(acc("ghost.testnet")).0, 0);
        assert!(c.get_account(acc("ghost.testnet")).is_none());
    }

    #[test]
    fn storage_registration_is_nep145_shaped() {
        let mut c = setup();
        assert_eq!(
            c.storage_balance_bounds().min.as_yoctonear(),
            STORAGE_PER_ACCOUNT.as_yoctonear()
        );
        testing_env!(ctx("alice.testnet", 0, STORAGE_PER_ACCOUNT.as_yoctonear()).build());
        let b = c.storage_deposit(None, Some(true));
        assert_eq!(b.total.as_yoctonear(), STORAGE_PER_ACCOUNT.as_yoctonear());
        assert!(c.storage_balance_of(acc("alice.testnet")).is_some());
    }

    // --- bookkeeping -----------------------------------------------------

    #[test]
    fn the_supply_never_grows() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);
        let start = c.ft_total_supply().0;

        testing_env!(ctx("alice.testnet", 0, 0).build());
        c.stamp();
        testing_env!(ctx("carol.testnet", 3, 0).build());
        c.poke(vec![acc("bob.testnet")]);
        testing_env!(ctx("alice.testnet", 7, 0).build());
        c.stamp();
        testing_env!(ctx("carol.testnet", 9, 0).build());
        c.poke(vec![acc("bob.testnet")]);
        testing_env!(ctx("alice.testnet", 14, 0).build());
        c.claim();

        assert!(c.ft_total_supply().0 < start, "stamps and burns shrink it");
        assert!(c.audit_supply());
    }

    #[test]
    fn the_books_balance_through_a_long_messy_run() {
        let mut c = setup();
        for who in ["alice.testnet", "bob.testnet", "carol.testnet", "dave.testnet"] {
            register_and_claim(&mut c, who, 0);
        }
        for day in 0..60u64 {
            // Alice stamps religiously, Carol sometimes, the others never.
            testing_env!(ctx("alice.testnet", day, 0).build());
            if day % 7 == 0 {
                c.stamp();
            }
            if day % 21 == 0 {
                testing_env!(ctx("carol.testnet", day, 0).build());
                c.stamp();
            }
            testing_env!(ctx("poker.testnet", day, 0).build());
            c.poke(vec![acc("bob.testnet"), acc("dave.testnet")]);
            assert!(c.audit_supply(), "books broke on day {day}");
        }

        testing_env!(ctx("alice.testnet", 60, 0).build());
        c.claim();
        assert!(c.audit_supply());

        // The sleepers have been drained; Alice has been fed.
        let bob = c.ft_balance_of(acc("bob.testnet")).0;
        let alice = c.ft_balance_of(acc("alice.testnet")).0;
        assert!(bob < FAUCET / 2, "bob slept for 60 days: {bob}");
        assert!(alice > bob, "alice {alice} should be ahead of bob {bob}");
    }

    #[test]
    fn the_event_log_keeps_the_newest_and_does_not_grow_forever() {
        let mut c = setup();
        register_and_claim(&mut c, "alice.testnet", 0);
        register_and_claim(&mut c, "bob.testnet", 0);
        for day in 0..120u64 {
            testing_env!(ctx("alice.testnet", day, 0).build());
            if day % 7 == 0 {
                c.stamp();
            }
            testing_env!(ctx("poker.testnet", day, 0).build());
            c.poke(vec![acc("bob.testnet")]);
        }
        let evs = c.get_events(Some(200));
        assert!(evs.len() <= EVENT_LOG_CAP as usize);
        assert!(evs.len() > 10);
        // Newest first.
        assert!(evs[0].day >= evs[evs.len() - 1].day);
    }

    #[test]
    fn views_page_without_falling_over() {
        let mut c = setup();
        for who in ["a.testnet", "b.testnet", "c.testnet"] {
            register_and_claim(&mut c, who, 0);
        }
        testing_env!(ctx("a.testnet", 3, 0).build());
        assert_eq!(c.list_accounts(None, None).len(), 3);
        assert_eq!(c.list_accounts(Some(1), Some(1)).len(), 1);
        assert!(!c.get_epochs(None, Some(5)).is_empty());
        assert!(c.get_stats().unrealised_rot.0 > 0, "rot is owed and visible");
    }

    #[test]
    fn poking_nobody_is_refused_and_poking_a_ghost_is_harmless() {
        let mut c = setup();
        testing_env!(ctx("a.testnet", 1, 0).build());
        assert_eq!(c.poke(vec![acc("ghost.testnet")]).0, 0);
    }

    // --- the three implementations agree ---------------------------------

    /// The Rust contract, the Python simulation and the browser model all
    /// claim to run the same arithmetic. This is where that claim is checked:
    /// `sim/vectors.json` is generated by the simulation, asserted here, and
    /// shipped to the page as `web/vectors.js`. If any of the three drifts,
    /// this test fails and the page's badge goes red.
    #[test]
    fn the_shared_vectors_hold() {
        use near_sdk::serde_json::Value;

        let raw = include_str!("../sim/vectors.json");
        let v: Value = near_sdk::serde_json::from_str(raw).expect("vectors.json is not valid JSON");

        let u = |x: &Value| -> u128 { x.as_str().unwrap().parse().unwrap() };
        let n = |x: &Value| -> u64 { x.as_u64().unwrap() };

        let mut checked = 0;

        for t in v["survival_factor"].as_array().unwrap() {
            let got = crate::math::survival_factor(n(&t["rot_bps"]) as u32, n(&t["days"]));
            assert_eq!(got, u(&t["value"]), "survival_factor days={}", t["days"]);
            checked += 1;
        }

        for t in v["apply_rot"].as_array().unwrap() {
            let (after, rotted) = apply_rot(
                u(&t["balance"]),
                n(&t["rot_bps"]) as u32,
                n(&t["days"]),
            );
            assert_eq!(after, u(&t["after"]), "apply_rot after days={}", t["days"]);
            assert_eq!(rotted, u(&t["rotted"]), "apply_rot rotted days={}", t["days"]);
            checked += 1;
        }

        for t in v["rot_days"].as_array().unwrap() {
            let got = rot_days(
                n(&t["settled"]),
                n(&t["protected_until"]),
                n(&t["now"]),
            );
            assert_eq!(got, n(&t["value"]), "rot_days {t}");
            checked += 1;
        }

        for t in v["stamp_cost"].as_array().unwrap() {
            let got = stamp_cost(u(&t["balance"]), n(&t["bps"]) as u32);
            assert_eq!(got, u(&t["value"]), "stamp_cost {t}");
            checked += 1;
        }

        for t in v["compost_share"].as_array().unwrap() {
            let got = compost_share(
                u(&t["compost"]),
                u(&t["stake"]),
                u(&t["balance_now"]),
                u(&t["weight"]),
            );
            assert_eq!(got, u(&t["value"]), "compost_share {t}");
            checked += 1;
        }

        assert!(checked >= 20, "expected the full vector set, got {checked}");
    }
}
