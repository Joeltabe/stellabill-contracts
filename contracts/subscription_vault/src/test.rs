use crate::{Subscription, SubscriptionStatus, SubscriptionVault, SubscriptionVaultClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{symbol_short, Address, Env, IntoVal};

#[test]
fn test_init_and_struct() {
    let env = Env::default();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    client.init(&token, &admin);
    // TODO: add create_subscription test with mock token
}

#[test]
fn test_subscription_struct() {
    let env = Env::default();
    let sub = Subscription {
        subscriber: Address::generate(&env),
        merchant: Address::generate(&env),
        amount: 10_000_0000, // 10 USDC (6 decimals)
        interval_seconds: 30 * 24 * 60 * 60, // 30 days
        last_payment_timestamp: 0,
        status: SubscriptionStatus::Active,
        prepaid_balance: 50_000_0000,
        usage_enabled: false,
    };
    assert_eq!(sub.status, SubscriptionStatus::Active);
}

#[test]
fn test_create_subscription_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    let amount = 10_000_0000i128;
    let interval = 2_592_000u64;
    
    let _sub_id = client.create_subscription(&subscriber, &merchant, &amount, &interval, &false);
    
    let events = env.events().all();
    let last_event = events.last().unwrap();
    
    assert_eq!(last_event.0, contract_id);
    assert_eq!(last_event.1, (symbol_short!("sub_new"),).into_val(&env));
}

#[test]
fn test_deposit_funds_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    
    let sub_id = client.create_subscription(&subscriber, &merchant, &10_000_0000, &2_592_000, &false);
    
    client.deposit_funds(&sub_id, &subscriber, &50_000_0000);
    
    let events = env.events().all();
    let last_event = events.last().unwrap();
    
    assert_eq!(last_event.0, contract_id);
    assert_eq!(last_event.1, (symbol_short!("deposit"),).into_val(&env));
}

#[test]
fn test_charge_subscription_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    let amount = 10_000_0000i128;
    
    let sub_id = client.create_subscription(&subscriber, &merchant, &amount, &2_592_000, &false);
    client.deposit_funds(&sub_id, &subscriber, &50_000_0000);
    
    client.charge_subscription(&sub_id);
    
    let events = env.events().all();
    let last_event = events.last().unwrap();
    
    assert_eq!(last_event.0, contract_id);
    assert_eq!(last_event.1, (symbol_short!("charged"),).into_val(&env));
}

#[test]
fn test_pause_subscription_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    
    let sub_id = client.create_subscription(&subscriber, &merchant, &10_000_0000, &2_592_000, &false);
    
    client.pause_subscription(&sub_id, &subscriber);
    
    let events = env.events().all();
    let last_event = events.last().unwrap();
    
    assert_eq!(last_event.0, contract_id);
    assert_eq!(last_event.1, (symbol_short!("paused"),).into_val(&env));
}

#[test]
fn test_resume_subscription_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    
    let sub_id = client.create_subscription(&subscriber, &merchant, &10_000_0000, &2_592_000, &false);
    client.pause_subscription(&sub_id, &subscriber);
    client.resume_subscription(&sub_id, &subscriber);
    
    let events = env.events().all();
    let last_event = events.last().unwrap();
    
    assert_eq!(last_event.0, contract_id);
    assert_eq!(last_event.1, (symbol_short!("resumed"),).into_val(&env));
}

#[test]
fn test_cancel_subscription_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    
    let sub_id = client.create_subscription(&subscriber, &merchant, &10_000_0000, &2_592_000, &false);
    
    client.cancel_subscription(&sub_id, &subscriber);
    
    let events = env.events().all();
    let last_event = events.last().unwrap();
    
    assert_eq!(last_event.0, contract_id);
    assert_eq!(last_event.1, (symbol_short!("cancelled"),).into_val(&env));
}

#[test]
fn test_withdraw_merchant_funds_emits_event() {
    let env = Env::default();
    env.mock_all_auths();
    
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    
    let merchant = Address::generate(&env);
    let amount = 100_000_0000i128;
    
    client.withdraw_merchant_funds(&merchant, &amount);
    
    let events = env.events().all();
    let last_event = events.last().unwrap();
    
    assert_eq!(last_event.0, contract_id);
    assert_eq!(last_event.1, (symbol_short!("withdraw"),).into_val(&env));
}

#[test]
fn test_full_lifecycle_events() {
    let env = Env::default();
    env.mock_all_auths();
    
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);
    
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    
    // Create
    let sub_id = client.create_subscription(&subscriber, &merchant, &10_000_0000, &2_592_000, &false);
    assert_eq!(env.events().all().last().unwrap().1, (symbol_short!("sub_new"),).into_val(&env));
    
    // Deposit
    client.deposit_funds(&sub_id, &subscriber, &50_000_0000);
    assert_eq!(env.events().all().last().unwrap().1, (symbol_short!("deposit"),).into_val(&env));
    
    // Charge
    client.charge_subscription(&sub_id);
    assert_eq!(env.events().all().last().unwrap().1, (symbol_short!("charged"),).into_val(&env));
    
    // Pause
    client.pause_subscription(&sub_id, &subscriber);
    assert_eq!(env.events().all().last().unwrap().1, (symbol_short!("paused"),).into_val(&env));
    
    // Resume
    client.resume_subscription(&sub_id, &subscriber);
    assert_eq!(env.events().all().last().unwrap().1, (symbol_short!("resumed"),).into_val(&env));
    
    // Cancel
    client.cancel_subscription(&sub_id, &subscriber);
    assert_eq!(env.events().all().last().unwrap().1, (symbol_short!("cancelled"),).into_val(&env));
}
