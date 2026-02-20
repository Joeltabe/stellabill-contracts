#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

#[contracterror]
#[repr(u32)]
pub enum Error {
    NotFound = 404,
    Unauthorized = 401,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionStatus {
    Active = 0,
    Paused = 1,
    Cancelled = 2,
    InsufficientBalance = 3,
}

/// Event emitted when a subscription is created.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SubscriptionCreatedEvent {
    pub subscription_id: u32,
    pub subscriber: Address,
    pub merchant: Address,
    pub amount: i128,
    pub interval_seconds: u64,
}

/// Event emitted when funds are deposited to a subscription.
#[contracttype]
#[derive(Clone, Debug)]
pub struct FundsDepositedEvent {
    pub subscription_id: u32,
    pub subscriber: Address,
    pub amount: i128,
    pub new_balance: i128,
}

/// Event emitted when a subscription is charged.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SubscriptionChargedEvent {
    pub subscription_id: u32,
    pub merchant: Address,
    pub amount: i128,
    pub remaining_balance: i128,
}

/// Event emitted when a subscription is paused.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SubscriptionPausedEvent {
    pub subscription_id: u32,
    pub authorizer: Address,
}

/// Event emitted when a subscription is resumed.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SubscriptionResumedEvent {
    pub subscription_id: u32,
    pub authorizer: Address,
}

/// Event emitted when a subscription is cancelled.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SubscriptionCancelledEvent {
    pub subscription_id: u32,
    pub authorizer: Address,
    pub refund_amount: i128,
}

/// Event emitted when a merchant withdraws funds.
#[contracttype]
#[derive(Clone, Debug)]
pub struct MerchantWithdrawalEvent {
    pub merchant: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Subscription {
    pub subscriber: Address,
    pub merchant: Address,
    pub amount: i128,
    pub interval_seconds: u64,
    pub last_payment_timestamp: u64,
    pub status: SubscriptionStatus,
    pub prepaid_balance: i128,
    pub usage_enabled: bool,
}

#[contract]
pub struct SubscriptionVault;

#[contractimpl]
impl SubscriptionVault {
    /// Initialize the contract (e.g. set token and admin). Extend as needed.
    pub fn init(env: Env, token: Address, admin: Address) -> Result<(), Error> {
        env.storage().instance().set(&Symbol::new(&env, "token"), &token);
        env.storage().instance().set(&Symbol::new(&env, "admin"), &admin);
        Ok(())
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
            merchant: merchant.clone(),
            amount,
            interval_seconds,
            last_payment_timestamp: env.ledger().timestamp(),
            status: SubscriptionStatus::Active,
            prepaid_balance: 0i128, // TODO: set from initial deposit
            usage_enabled,
        };
        let id = Self::_next_id(&env);
        env.storage().instance().set(&id, &sub);
        
        env.events().publish(
            (symbol_short!("sub_new"),),
            SubscriptionCreatedEvent {
                subscription_id: id,
                subscriber,
                merchant,
                amount,
                interval_seconds,
            },
        );
        
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
        // TODO: transfer USDC from subscriber, increase prepaid_balance for subscription_id
        let mut sub: Subscription = env.storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)?;
        
        sub.prepaid_balance += amount;
        env.storage().instance().set(&subscription_id, &sub);
        
        env.events().publish(
            (symbol_short!("deposit"),),
            FundsDepositedEvent {
                subscription_id,
                subscriber,
                amount,
                new_balance: sub.prepaid_balance,
            },
        );
        
        Ok(())
    }

    /// Billing engine (backend) calls this to charge one interval. Deducts from vault, pays merchant.
    pub fn charge_subscription(env: Env, subscription_id: u32) -> Result<(), Error> {
        // TODO: require_caller admin or authorized billing service
        let mut sub: Subscription = env.storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)?;
        
        // TODO: transfer to merchant, check balance
        sub.prepaid_balance -= sub.amount;
        sub.last_payment_timestamp = env.ledger().timestamp();
        env.storage().instance().set(&subscription_id, &sub);
        
        env.events().publish(
            (symbol_short!("charged"),),
            SubscriptionChargedEvent {
                subscription_id,
                merchant: sub.merchant.clone(),
                amount: sub.amount,
                remaining_balance: sub.prepaid_balance,
            },
        );
        
        Ok(())
    }

    /// Subscriber or merchant cancels the subscription. Remaining balance can be withdrawn by subscriber.
    pub fn cancel_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();
        let mut sub: Subscription = env.storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)?;
        
        // TODO: allow withdraw of prepaid_balance
        sub.status = SubscriptionStatus::Cancelled;
        let refund = sub.prepaid_balance;
        env.storage().instance().set(&subscription_id, &sub);
        
        env.events().publish(
            (symbol_short!("cancelled"),),
            SubscriptionCancelledEvent {
                subscription_id,
                authorizer,
                refund_amount: refund,
            },
        );
        
        Ok(())
    }

    /// Pause subscription (no charges until resumed).
    pub fn pause_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();
        let mut sub: Subscription = env.storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)?;
        
        sub.status = SubscriptionStatus::Paused;
        env.storage().instance().set(&subscription_id, &sub);
        
        env.events().publish(
            (symbol_short!("paused"),),
            SubscriptionPausedEvent {
                subscription_id,
                authorizer,
            },
        );
        
        Ok(())
    }

    /// Resume a paused subscription.
    pub fn resume_subscription(
        env: Env,
        subscription_id: u32,
        authorizer: Address,
    ) -> Result<(), Error> {
        authorizer.require_auth();
        let mut sub: Subscription = env.storage()
            .instance()
            .get(&subscription_id)
            .ok_or(Error::NotFound)?;
        
        sub.status = SubscriptionStatus::Active;
        env.storage().instance().set(&subscription_id, &sub);
        
        env.events().publish(
            (symbol_short!("resumed"),),
            SubscriptionResumedEvent {
                subscription_id,
                authorizer,
            },
        );
        
        Ok(())
    }

    /// Merchant withdraws accumulated USDC to their wallet.
    pub fn withdraw_merchant_funds(
        env: Env,
        merchant: Address,
        amount: i128,
    ) -> Result<(), Error> {
        merchant.require_auth();
        // TODO: deduct from merchant's balance in contract, transfer token to merchant
        
        env.events().publish(
            (symbol_short!("withdraw"),),
            MerchantWithdrawalEvent {
                merchant,
                amount,
            },
        );
        
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
