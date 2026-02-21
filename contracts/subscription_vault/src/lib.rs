#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol};

#[contracterror]
#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    NotFound = 404,
    Unauthorized = 401,
    /// Charge attempted before `last_payment_timestamp + interval_seconds`.
    IntervalNotElapsed = 1001,
    /// Subscription is not Active (e.g. Paused, Cancelled).
    NotActive = 1002,
    /// `charge_usage` called on a subscription with `usage_enabled = false`.
    UsageNotEnabled = 1003,
    /// Prepaid balance is too low to cover the requested debit.
    InsufficientPrepaidBalance = 1004,
    /// The supplied amount is not a positive value.
    InvalidAmount = 1005,
    InvalidStatusTransition = 400,
    BelowMinimumTopup = 402,
}

/// Represents the lifecycle state of a subscription.
///
/// # State Machine
///
/// The subscription status follows a defined state machine with specific allowed transitions:
///
/// - **Active**: Subscription is active and charges can be processed.
///   - Can transition to: `Paused`, `Cancelled`, `InsufficientBalance`
///
/// - **Paused**: Subscription is temporarily suspended, no charges are processed.
///   - Can transition to: `Active`, `Cancelled`
///
/// - **Cancelled**: Subscription is permanently terminated, no further changes allowed.
///   - No outgoing transitions (terminal state)
///
/// - **InsufficientBalance**: Subscription failed due to insufficient funds.
///   - Can transition to: `Active` (after deposit), `Cancelled`
///
/// Invalid transitions (e.g., `Cancelled` -> `Active`) are rejected with
/// [`Error::InvalidStatusTransition`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionStatus {
    /// Subscription is active and ready for charging.
    Active = 0,
    /// Subscription is temporarily paused, no charges processed.
    Paused = 1,
    /// Subscription is permanently cancelled (terminal state).
    Cancelled = 2,
    /// Subscription failed due to insufficient balance for charging.
    InsufficientBalance = 3,
}

/// Stores subscription details and current state.
///
/// The `status` field is managed by the state machine. Use the provided
/// transition helpers to modify status, never set it directly.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Subscription {
    pub subscriber: Address,
    pub merchant: Address,
    pub amount: i128,
    pub interval_seconds: u64,
    pub last_payment_timestamp: u64,
    /// Current lifecycle state. Modified only through state machine transitions.
    pub status: SubscriptionStatus,
    pub prepaid_balance: i128,
    pub usage_enabled: bool,
}

/// Validates if a status transition is allowed by the state machine.
///
/// # State Transition Rules
///
/// | From              | To                  | Allowed |
/// |-------------------|---------------------|---------|
/// | Active            | Paused              | Yes     |
/// | Active            | Cancelled           | Yes     |
/// | Active            | InsufficientBalance | Yes     |
/// | Paused            | Active              | Yes     |
/// | Paused            | Cancelled           | Yes     |
/// | InsufficientBalance | Active            | Yes     |
/// | InsufficientBalance | Cancelled         | Yes     |
/// | Cancelled         | *any*               | No      |
/// | *any*             | Same status         | Yes (idempotent) |
///
/// # Arguments
/// * `from` - Current status
/// * `to` - Target status
///
/// # Returns
/// * `Ok(())` if transition is valid
/// * `Err(Error::InvalidStatusTransition)` if transition is invalid
pub fn validate_status_transition(
    from: &SubscriptionStatus,
    to: &SubscriptionStatus,
) -> Result<(), Error> {
    // Same status is always allowed (idempotent)
    if from == to {
        return Ok(());
    }

    let valid = match from {
        SubscriptionStatus::Active => matches!(
            to,
            SubscriptionStatus::Paused
                | SubscriptionStatus::Cancelled
                | SubscriptionStatus::InsufficientBalance
        ),
        SubscriptionStatus::Paused => {
            matches!(to, SubscriptionStatus::Active | SubscriptionStatus::Cancelled)
        }
        SubscriptionStatus::Cancelled => false,
        SubscriptionStatus::InsufficientBalance => {
            matches!(to, SubscriptionStatus::Active | SubscriptionStatus::Cancelled)
        }
    };

    if valid {
        Ok(())
    } else {
        Err(Error::InvalidStatusTransition)
    }
}

/// Returns all valid target statuses for a given current status.
///
/// This is useful for UI/documentation to show available actions.
///
/// # Examples
///
/// ```
/// let targets = get_allowed_transitions(&SubscriptionStatus::Active);
/// assert!(targets.contains(&SubscriptionStatus::Paused));
/// ```
pub fn get_allowed_transitions(status: &SubscriptionStatus) -> &'static [SubscriptionStatus] {
    match status {
        SubscriptionStatus::Active => &[
            SubscriptionStatus::Paused,
            SubscriptionStatus::Cancelled,
            SubscriptionStatus::InsufficientBalance,
        ],
        SubscriptionStatus::Paused => &[
            SubscriptionStatus::Active,
            SubscriptionStatus::Cancelled,
        ],
        SubscriptionStatus::Cancelled => &[],
        SubscriptionStatus::InsufficientBalance => &[
            SubscriptionStatus::Active,
            SubscriptionStatus::Cancelled,
        ],
    }
}

/// Checks if a transition is valid without returning an error.
///
/// Convenience wrapper around [`validate_status_transition`] for boolean checks.
pub fn can_transition(from: &SubscriptionStatus, to: &SubscriptionStatus) -> bool {
    validate_status_transition(from, to).is_ok()
}

#[contract]
pub struct SubscriptionVault;

#[contractimpl]
impl SubscriptionVault {
    /// Initialize the contract (e.g. set token and admin). Extend as needed.
    pub fn init(env: Env, token: Address, admin: Address, min_topup: i128) -> Result<(), Error> {
        env.storage().instance().set(&Symbol::new(&env, "token"), &token);
        env.storage().instance().set(&Symbol::new(&env, "admin"), &admin);
        env.storage().instance().set(&Symbol::new(&env, "min_topup"), &min_topup);
        Ok(())
    }

    /// Update the minimum top-up threshold. Only callable by admin.
    /// 
    /// # Arguments
    /// * `min_topup` - Minimum amount (in token base units) required for deposit_funds.
    ///                 Prevents inefficient micro-deposits. Typical range: 1-10 USDC (1_000000 - 10_000000 for 6 decimals).
    pub fn set_min_topup(env: Env, admin: Address, min_topup: i128) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&Symbol::new(&env, "admin")).ok_or(Error::NotFound)?;
        if admin != stored_admin {
            return Err(Error::Unauthorized);
        }
        env.storage().instance().set(&Symbol::new(&env, "min_topup"), &min_topup);
        Ok(())
    }

    /// Get the current minimum top-up threshold.
    pub fn get_min_topup(env: Env) -> Result<i128, Error> {
        env.storage().instance().get(&Symbol::new(&env, "min_topup")).ok_or(Error::NotFound)
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
        // TODO: transfer initial deposit from subscriber to contract, then store subscription
        let sub = Subscription {
            subscriber: subscriber.clone(),
            merchant,
            amount,
            interval_seconds,
            last_payment_timestamp: env.ledger().timestamp(),
            status: SubscriptionStatus::Active,
            prepaid_balance: 0i128, // TODO: set from initial deposit
            usage_enabled,
        };
        let id = Self::_next_id(&env);
        env.storage().instance().set(&id, &sub);
        Ok(id)
    }

    /// Subscriber deposits more USDC into their vault for this subscription.
    /// 
    /// # Minimum top-up enforcement
    /// Rejects deposits below the configured minimum threshold to prevent inefficient
    /// micro-transactions that waste gas and complicate accounting. The minimum is set
    /// globally at contract initialization and adjustable by admin via `set_min_topup`.
    pub fn deposit_funds(
        env: Env,
        subscription_id: u32,
        subscriber: Address,
        amount: i128,
    ) -> Result<(), Error> {
        subscriber.require_auth();
        
        let min_topup: i128 = env.storage().instance().get(&Symbol::new(&env, "min_topup")).ok_or(Error::NotFound)?;
        if amount < min_topup {
            return Err(Error::BelowMinimumTopup);
        }
        
        // TODO: transfer USDC from subscriber, increase prepaid_balance for subscription_id
        let _ = (env, subscription_id, amount);
        Ok(())
    }

    /// Billing engine (backend) calls this to charge one interval.
    ///
    /// Enforces strict interval timing: the current ledger timestamp must be
    /// >= `last_payment_timestamp + interval_seconds`. If the interval has not
    /// elapsed, returns `Error::IntervalNotElapsed` and leaves storage unchanged.
    /// On success, `last_payment_timestamp` is advanced to the current ledger
    /// timestamp.
    pub fn charge_subscription(env: Env, subscription_id: u32) -> Result<(), Error> {
        // TODO: require_caller admin or authorized billing service

        let mut sub: Subscription = env
            .storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)?;

        if sub.status != SubscriptionStatus::Active {
            return Err(Error::NotActive);
        }

        let now = env.ledger().timestamp();
        let next_charge_at = sub
            .last_payment_timestamp
            .checked_add(sub.interval_seconds)
            .expect("interval overflow");

        if now < next_charge_at {
            return Err(Error::IntervalNotElapsed);
        }

        sub.last_payment_timestamp = now;

        // TODO: deduct sub.amount from sub.prepaid_balance, transfer to merchant

        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// Charge a metered usage amount against the subscription's prepaid balance.
    ///
    /// Designed for integration with an **off-chain usage metering service**:
    /// the service measures consumption, then calls this entrypoint with the
    /// computed `usage_amount` to debit the subscriber's vault.
    ///
    /// # Requirements
    ///
    /// * The subscription must be `Active`.
    /// * `usage_enabled` must be `true` on the subscription.
    /// * `usage_amount` must be positive (`> 0`).
    /// * `prepaid_balance` must be >= `usage_amount`.
    ///
    /// # Behaviour
    ///
    /// On success, `prepaid_balance` is reduced by `usage_amount`.  If the
    /// debit drains the balance to zero the subscription transitions to
    /// `InsufficientBalance` status, signalling that no further charges
    /// (interval or usage) can proceed until the subscriber tops up.
    ///
    /// # Errors
    ///
    /// | Variant | Reason |
    /// |---------|--------|
    /// | `NotFound` | Subscription ID does not exist. |
    /// | `NotActive` | Subscription is not `Active`. |
    /// | `UsageNotEnabled` | `usage_enabled` is `false`. |
    /// | `InvalidAmount` | `usage_amount` is zero or negative. |
    /// | `InsufficientPrepaidBalance` | Prepaid balance cannot cover the debit. |
    pub fn charge_usage(
        env: Env,
        subscription_id: u32,
        usage_amount: i128,
    ) -> Result<(), Error> {
        // TODO: require_caller admin or authorized metering service

        let mut sub: Subscription = env
            .storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)?;

        if sub.status != SubscriptionStatus::Active {
            return Err(Error::NotActive);
        }

        if !sub.usage_enabled {
            return Err(Error::UsageNotEnabled);
        }

        if usage_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if sub.prepaid_balance < usage_amount {
            return Err(Error::InsufficientPrepaidBalance);
        }

        sub.prepaid_balance -= usage_amount;

        // If the vault is now empty, transition to InsufficientBalance so no
        // further charges (interval or usage) can proceed until top-up.
        if sub.prepaid_balance == 0 {
            sub.status = SubscriptionStatus::InsufficientBalance;
        }

        // TODO: transfer usage_amount USDC to merchant

        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// Subscriber or merchant cancels the subscription. Remaining balance can be withdrawn by subscriber.
    ///
    /// # State Transitions
    /// Allowed from: `Active`, `Paused`, `InsufficientBalance`
    /// - Transitions to: `Cancelled` (terminal state)
    ///
    /// Once cancelled, no further transitions are possible.
    pub fn cancel_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();

        let mut sub = Self::get_subscription(env.clone(), subscription_id)?;

        // Validate and apply status transition
        validate_status_transition(&sub.status, &SubscriptionStatus::Cancelled)?;
        sub.status = SubscriptionStatus::Cancelled;

        // TODO: allow withdraw of prepaid_balance

        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// Pause subscription (no charges until resumed).
    ///
    /// # State Transitions
    /// Allowed from: `Active`
    /// - Transitions to: `Paused`
    ///
    /// Cannot pause a subscription that is already `Paused`, `Cancelled`, or in `InsufficientBalance`.
    pub fn pause_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();

        let mut sub = Self::get_subscription(env.clone(), subscription_id)?;

        // Validate and apply status transition
        validate_status_transition(&sub.status, &SubscriptionStatus::Paused)?;
        sub.status = SubscriptionStatus::Paused;

        env.storage().instance().set(&subscription_id, &sub);
        Ok(())
    }

    /// Resume a subscription to Active status.
    ///
    /// # State Transitions
    /// Allowed from: `Paused`, `InsufficientBalance`
    /// - Transitions to: `Active`
    ///
    /// Cannot resume a `Cancelled` subscription.
    pub fn resume_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();

        let mut sub = Self::get_subscription(env.clone(), subscription_id)?;

        // Validate and apply status transition
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
        // TODO: deduct from merchant's balance in contract, transfer token to merchant
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
