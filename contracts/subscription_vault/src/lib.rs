#![no_std]

mod state_machine;
mod types;

use soroban_sdk::{contract, contractimpl, Address, Env, Symbol, Vec};

pub use state_machine::{can_transition, get_allowed_transitions, validate_status_transition};
pub use types::{BatchChargeResult, Error, Subscription, SubscriptionStatus};

#[contract]
pub struct SubscriptionVault;

#[contractimpl]
impl SubscriptionVault {
    /// Initialize the contract (e.g. set token and admin). Extend as needed.
    pub fn init(env: Env, token: Address, admin: Address, min_topup: i128) -> Result<(), Error> {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "token"), &token);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "admin"), &admin);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "min_topup"), &min_topup);
        Ok(())
    }

    /// Update the minimum top-up threshold. Only callable by admin.
    pub fn set_min_topup(env: Env, admin: Address, min_topup: i128) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "admin"))
            .ok_or(Error::NotFound)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "min_topup"), &min_topup);
        Ok(())
    }

    /// Get the current minimum top-up threshold.
    pub fn get_min_topup(env: Env) -> Result<i128, Error> {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "min_topup"))
            .ok_or(Error::NotFound)
    }

    /// Create a new subscription. Caller deposits initial USDC; contract stores agreement.
    pub fn create_subscription(
        env: Env,
        subscriber: Address,
        merchant: Address,
        amount: i128,
        interval_seconds: u64,
        usage_enabled: bool,
    ) -> Result<u32, Error> {
        subscriber.require_auth();
        let sub = Subscription {
            subscriber: subscriber.clone(),
            merchant,
            amount,
            interval_seconds,
            last_payment_timestamp: env.ledger().timestamp(),
            status: SubscriptionStatus::Active,
            prepaid_balance: 0i128,
            usage_enabled,
        };
        let id = Self::_next_id(&env);
        env.storage().instance().set(&id, &sub);
        Ok(id)
    }

    /// Subscriber deposits more USDC into their vault for this subscription.
    pub fn deposit_funds(
        env: Env,
        subscription_id: u32,
        subscriber: Address,
        amount: i128,
    ) -> Result<(), Error> {
        subscriber.require_auth();

        let min_topup: i128 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "min_topup"))
            .ok_or(Error::NotFound)?;
        if amount < min_topup {
            return Err(Error::BelowMinimumTopup);
        }

        let mut sub = Self::get_subscription(env.clone(), subscription_id)?;
        sub.prepaid_balance = sub
            .prepaid_balance
            .checked_add(amount)
            .ok_or(Error::Overflow)?;
        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// Billing engine (backend) calls this to charge one interval. Deducts from vault, pays merchant.
    ///
    /// Only the authorized admin or billing engine address set during `init` can invoke this.
    /// Fails with `Error::Unauthorized` if the caller is not the stored admin.
    ///
    /// # State Transitions
    /// - On success: `Active` -> `Active` (no change)
    /// - On insufficient balance: `Active` -> `InsufficientBalance`
    ///
    /// Subscriptions that are `Paused` or `Cancelled` cannot be charged.
    pub fn charge_subscription(env: Env, subscription_id: u32) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "admin"))
            .ok_or(Error::Unauthorized)?;
        admin.require_auth();
        Self::_charge_one(env, subscription_id)
    }

    /// Internal: perform one charge (no auth). Used by charge_subscription and batch_charge.
    fn _charge_one(env: Env, subscription_id: u32) -> Result<(), Error> {
        let mut sub = Self::get_subscription(env.clone(), subscription_id)?;

        if sub.status != SubscriptionStatus::Active {
            return Err(Error::NotActive);
        }

        let now = env.ledger().timestamp();
        let next_allowed = sub
            .last_payment_timestamp
            .checked_add(sub.interval_seconds)
            .ok_or(Error::Overflow)?;
        if now < next_allowed {
            return Err(Error::IntervalNotElapsed);
        }

        if sub.prepaid_balance < sub.amount {
            validate_status_transition(&sub.status, &SubscriptionStatus::InsufficientBalance)?;
            sub.status = SubscriptionStatus::InsufficientBalance;
            env.storage().instance().set(&subscription_id, &sub);
            return Err(Error::InsufficientBalance);
        }

        sub.prepaid_balance = sub
            .prepaid_balance
            .checked_sub(sub.amount)
            .ok_or(Error::Overflow)?;
        sub.last_payment_timestamp = now;
        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// **Read-only.** Estimates how much additional prepaid balance is required to cover
    /// a specified number of future billing intervals.
    ///
    /// # Arguments
    /// * `subscription_id` - The subscription to evaluate.
    /// * `num_intervals` - Number of future intervals to cover (e.g. 3 for “next 3 charges”).
    ///
    /// # Returns
    /// * `Ok(amount)` - Additional amount (in token base units) the subscriber should top up.
    ///   Zero if current `prepaid_balance` already covers `num_intervals` (or more).
    /// * `Err(Error::NotFound)` - Subscription does not exist.
    /// * `Err(Error::Overflow)` - `amount * num_intervals` would overflow (safe math).
    ///
    /// # Edge cases
    /// * **Zero intervals:** returns `Ok(0)` (no top-up needed).
    /// * **Insufficient balance:** returns the shortfall (positive amount to add).
    /// * **Balance already sufficient:** returns `0`.
    ///
    /// # Example (UI)
    /// ```ignore
    /// let topup = contract.estimate_topup_for_intervals(&sub_id, &3);
    /// if topup > 0 { show "Add X USDC to cover the next 3 payments" }
    /// ```
    pub fn estimate_topup_for_intervals(
        env: Env,
        subscription_id: u32,
        num_intervals: u32,
    ) -> Result<i128, Error> {
        let sub = Self::get_subscription(env, subscription_id)?;

        if num_intervals == 0 {
            return Ok(0);
        }

        let intervals_i128: i128 = num_intervals.into();
        let required = sub
            .amount
            .checked_mul(intervals_i128)
            .ok_or(Error::Overflow)?;

        let topup = required
            .checked_sub(sub.prepaid_balance)
            .unwrap_or(0)
            .max(0);
        Ok(topup)
    }

    /// Admin-only batch charge: charge multiple subscriptions in one transaction.
    ///
    /// Uses the same admin authorization as single `charge_subscription`. Each subscription
    /// is charged independently; partial failures do not roll back successful charges.
    ///
    /// # Arguments
    /// * `subscription_ids` - List of subscription IDs to charge (order is preserved in results).
    ///
    /// # Returns
    /// * `Ok(results)` - One [`BatchChargeResult`] per subscription (same length as input).
    /// * Caller must be the stored admin (same as single charge).
    ///
    /// # Semantics
    /// * Empty list: returns empty Vec.
    /// * Duplicate IDs: each is processed once (duplicates may succeed or fail independently).
    /// * Per-item errors (e.g. IntervalNotElapsed, NotActive, insufficient balance) are
    ///   returned in the corresponding slot; other subscriptions are still charged.
    pub fn batch_charge(
        env: Env,
        subscription_ids: Vec<u32>,
    ) -> Result<Vec<BatchChargeResult>, Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "admin"))
            .ok_or(Error::Unauthorized)?;
        admin.require_auth();

        let mut results = Vec::new(&env);
        for id in subscription_ids.iter() {
            let r = Self::_charge_one(env.clone(), id);
            let res = match &r {
                Ok(()) => BatchChargeResult {
                    success: true,
                    error_code: 0,
                },
                Err(e) => BatchChargeResult {
                    success: false,
                    error_code: e.clone().to_code(),
                },
            };
            results.push_back(res);
        }
        Ok(results)
    }

    /// Subscriber or merchant cancels the subscription. Remaining balance can be withdrawn by subscriber.
    pub fn cancel_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();

        let mut sub = Self::get_subscription(env.clone(), subscription_id)?;

        validate_status_transition(&sub.status, &SubscriptionStatus::Cancelled)?;
        sub.status = SubscriptionStatus::Cancelled;

        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// Pause subscription (no charges until resumed).
    pub fn pause_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();

        let mut sub = Self::get_subscription(env.clone(), subscription_id)?;

        validate_status_transition(&sub.status, &SubscriptionStatus::Paused)?;
        sub.status = SubscriptionStatus::Paused;

        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// Resume a subscription to Active status.
    pub fn resume_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();

        let mut sub = Self::get_subscription(env.clone(), subscription_id)?;

        validate_status_transition(&sub.status, &SubscriptionStatus::Active)?;
        sub.status = SubscriptionStatus::Active;

        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// Merchant withdraws accumulated USDC to their wallet.
    pub fn withdraw_merchant_funds(
        _env: Env,
        merchant: Address,
        _amount: i128,
    ) -> Result<(), Error> {
        merchant.require_auth();
        Ok(())
    }

    /// Read subscription by id (for indexing and UI).
    pub fn get_subscription(env: Env, subscription_id: u32) -> Result<Subscription, Error> {
        env.storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)
    }

    fn _next_id(env: &Env) -> u32 {
        let key = Symbol::new(env, "next_id");
        let id: u32 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage().instance().set(&key, &(id + 1));
        id
    }
}

#[cfg(test)]
mod test;
