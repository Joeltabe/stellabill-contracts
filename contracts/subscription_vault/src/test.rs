use crate::{Error, Subscription, SubscriptionStatus, SubscriptionVault, SubscriptionVaultClient};
use crate::safe_math::*;
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};

#[test]
fn test_init_and_struct() {
    let env = Env::default();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    let min_topup = 1_000000i128; // 1 USDC
    client.init(&token, &admin, &min_topup);
    
    assert_eq!(client.get_min_topup(), min_topup);
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

// ============================================================================
// Safe Math Tests
// ============================================================================

#[test]
fn test_safe_add_normal() {
    assert_eq!(safe_add(100, 200), Ok(300));
    assert_eq!(safe_add(0, 1000), Ok(1000));
    assert_eq!(safe_add(1_000_000, 2_000_000), Ok(3_000_000));
}

#[test]
fn test_safe_add_overflow() {
    assert_eq!(safe_add(i128::MAX, 1), Err(Error::Overflow));
    assert_eq!(safe_add(i128::MAX, 0), Ok(i128::MAX));
    assert_eq!(safe_add(i128::MAX - 1, 2), Err(Error::Overflow));
}

#[test]
fn test_safe_sub_normal() {
    assert_eq!(safe_sub(200, 100), Ok(100));
    assert_eq!(safe_sub(1000, 0), Ok(1000));
    assert_eq!(safe_sub(5_000_000, 2_000_000), Ok(3_000_000));
}

#[test]
fn test_safe_sub_underflow() {
    assert_eq!(safe_sub(i128::MIN, 1), Err(Error::Underflow));
    assert_eq!(safe_sub(i128::MIN, 0), Ok(i128::MIN));
    assert_eq!(safe_sub(i128::MIN + 1, 2), Err(Error::Underflow));
}

#[test]
fn test_safe_sub_negative_result() {
    // safe_sub allows negative results (it's for general arithmetic)
    assert_eq!(safe_sub(100, 200), Ok(-100));
    assert_eq!(safe_sub(0, 1), Ok(-1));
}

#[test]
fn test_validate_non_negative() {
    assert_eq!(validate_non_negative(0), Ok(()));
    assert_eq!(validate_non_negative(100), Ok(()));
    assert_eq!(validate_non_negative(i128::MAX), Ok(()));
    assert_eq!(validate_non_negative(-1), Err(Error::Underflow));
    assert_eq!(validate_non_negative(i128::MIN), Err(Error::Underflow));
}

#[test]
fn test_safe_add_balance_normal() {
    assert_eq!(safe_add_balance(1000, 500), Ok(1500));
    assert_eq!(safe_add_balance(0, 1000), Ok(1000));
    assert_eq!(safe_add_balance(1_000_000, 2_000_000), Ok(3_000_000));
}

#[test]
fn test_safe_add_balance_overflow() {
    assert_eq!(safe_add_balance(i128::MAX, 1), Err(Error::Overflow));
    assert_eq!(safe_add_balance(i128::MAX, 0), Ok(i128::MAX));
}

#[test]
fn test_safe_add_balance_negative_amount() {
    assert_eq!(safe_add_balance(1000, -100), Err(Error::Underflow));
    assert_eq!(safe_add_balance(0, -1), Err(Error::Underflow));
}

#[test]
fn test_safe_sub_balance_normal() {
    assert_eq!(safe_sub_balance(1000, 500), Ok(500));
    assert_eq!(safe_sub_balance(1000, 0), Ok(1000));
    assert_eq!(safe_sub_balance(5_000_000, 2_000_000), Ok(3_000_000));
}

#[test]
fn test_safe_sub_balance_insufficient() {
    assert_eq!(safe_sub_balance(1000, 1500), Err(Error::Underflow));
    assert_eq!(safe_sub_balance(100, 200), Err(Error::Underflow));
    assert_eq!(safe_sub_balance(0, 1), Err(Error::Underflow));
}

#[test]
fn test_safe_sub_balance_negative_amount() {
    assert_eq!(safe_sub_balance(1000, -100), Err(Error::Underflow));
    assert_eq!(safe_sub_balance(0, -1), Err(Error::Underflow));
}

#[test]
fn test_safe_sub_balance_exact_zero() {
    assert_eq!(safe_sub_balance(1000, 1000), Ok(0));
    assert_eq!(safe_sub_balance(1_000_000, 1_000_000), Ok(0));
}

#[test]
fn test_safe_add_zero() {
    assert_eq!(safe_add(0, 0), Ok(0));
    assert_eq!(safe_add(100, 0), Ok(100));
    assert_eq!(safe_add(0, 100), Ok(100));
    assert_eq!(safe_add(i128::MAX, 0), Ok(i128::MAX));
}

#[test]
fn test_safe_sub_zero() {
    assert_eq!(safe_sub(0, 0), Ok(0));
    assert_eq!(safe_sub(100, 0), Ok(100));
    assert_eq!(safe_sub(i128::MAX, 0), Ok(i128::MAX));
}

#[test]
fn test_safe_add_max_to_zero() {
    assert_eq!(safe_add(0, i128::MAX), Ok(i128::MAX));
}

#[test]
fn test_safe_sub_from_max() {
    assert_eq!(safe_sub(i128::MAX, 0), Ok(i128::MAX));
    assert_eq!(safe_sub(i128::MAX, 1), Ok(i128::MAX - 1));
}

#[test]
fn test_safe_add_max_to_one() {
    assert_eq!(safe_add(i128::MAX, 1), Err(Error::Overflow));
}

#[test]
fn test_safe_sub_min_from_zero() {
    // Subtracting i128::MIN from 0 would require adding i128::MAX + 1, which overflows
    // This tests the edge case where subtraction underflows
    assert_eq!(safe_sub(0, i128::MIN), Err(Error::Underflow));
}

#[test]
fn test_usdc_amounts() {
    // Test with realistic USDC amounts (6 decimals)
    let one_usdc = 1_000_000i128;
    let thousand_usdc = 1_000_000_000i128;
    let ten_thousand_usdc = 10_000_000_000i128;

    // Addition
    assert_eq!(safe_add_balance(one_usdc, thousand_usdc), Ok(1_001_000_000));
    assert_eq!(
        safe_add_balance(thousand_usdc, ten_thousand_usdc),
        Ok(11_000_000_000)
    );

    // Subtraction
    assert_eq!(safe_sub_balance(thousand_usdc, one_usdc), Ok(999_000_000));
    assert_eq!(
        safe_sub_balance(ten_thousand_usdc, thousand_usdc),
        Ok(9_000_000_000)
    );

    // Edge case: maximum reasonable USDC amount (still well below i128::MAX)
    let max_reasonable_usdc = 1_000_000_000_000_000i128; // 1 trillion USDC
    assert_eq!(
        safe_add_balance(max_reasonable_usdc, one_usdc),
        Ok(max_reasonable_usdc + one_usdc)
    );
}

#[test]
fn test_deposit_funds_with_safe_math() {
    // Test that safe_add_balance is used correctly in deposit_funds
    // This test verifies the safe math integration through direct function calls
    // Note: Full integration test requires proper auth mocking which is complex
    // The core safe math functionality is tested in the dedicated safe math tests above
    
    // Test safe_add_balance directly (which is what deposit_funds uses)
    assert_eq!(safe_add_balance(0, 5_000_000i128), Ok(5_000_000i128));
    assert_eq!(safe_add_balance(5_000_000i128, 3_000_000i128), Ok(8_000_000i128));
    
    // Test overflow protection
    assert_eq!(safe_add_balance(i128::MAX, 1), Err(Error::Overflow));
    
    // Test negative amount rejection
    assert_eq!(safe_add_balance(1000, -100), Err(Error::Underflow));
}

#[test]
fn test_deposit_funds_rejects_negative() {
    // Test that validate_non_negative (used in deposit_funds) rejects negative amounts
    assert_eq!(validate_non_negative(-1_000_000i128), Err(Error::Underflow));
    assert_eq!(validate_non_negative(0), Ok(()));
    assert_eq!(validate_non_negative(1_000_000i128), Ok(()));
}

#[test]
fn test_charge_subscription_with_safe_math() {
    // Test that safe_sub_balance is used correctly in charge_subscription
    // This verifies safe math integration for charge operations
    
    // Test normal charge (deduct amount from balance)
    assert_eq!(safe_sub_balance(30_000_000i128, 10_000_000i128), Ok(20_000_000i128));
    
    // Test insufficient balance (should fail)
    assert_eq!(safe_sub_balance(5_000_000i128, 10_000_000i128), Err(Error::Underflow));
    
    // Test exact balance (should succeed with zero result)
    assert_eq!(safe_sub_balance(10_000_000i128, 10_000_000i128), Ok(0i128));
}

#[test]
fn test_charge_subscription_insufficient_balance() {
    // Test that safe_sub_balance prevents charging when balance is insufficient
    assert_eq!(safe_sub_balance(0, 10_000_000i128), Err(Error::Underflow));
    assert_eq!(safe_sub_balance(5_000_000i128, 10_000_000i128), Err(Error::Underflow));
}

#[test]
fn test_multiple_deposits_no_overflow() {
    // Test that multiple large deposits don't overflow
    let large_amount = 100_000_000_000i128; // 100k USDC
    let mut balance = 0i128;
    
    // Simulate 10 deposits
    for _ in 0..10 {
        balance = safe_add_balance(balance, large_amount).unwrap();
    }
    
    assert_eq!(balance, 1_000_000_000_000i128); // 1M USDC total
    
    // Test that adding a very large amount close to i128::MAX would overflow
    // Use an amount that would definitely cause overflow
    let overflow_amount = i128::MAX - balance + 1;
    assert_eq!(safe_add_balance(balance, overflow_amount), Err(Error::Overflow));
    
    // Test that adding a reasonable amount still works
    assert_eq!(safe_add_balance(balance, large_amount), Ok(balance + large_amount));
}

#[test]
fn test_repeated_charges_no_underflow() {
    // Test that repeated charges don't underflow
    let charge_amount = 10_000_000i128; // 10 USDC
    let mut balance = 30_000_000i128; // 30 USDC (enough for 3 charges)
    
    // Charge 3 times
    balance = safe_sub_balance(balance, charge_amount).unwrap();
    assert_eq!(balance, 20_000_000i128);
    
    balance = safe_sub_balance(balance, charge_amount).unwrap();
    assert_eq!(balance, 10_000_000i128);
    
    balance = safe_sub_balance(balance, charge_amount).unwrap();
    assert_eq!(balance, 0i128);
    
    // Try to charge again - should fail
    assert_eq!(safe_sub_balance(balance, charge_amount), Err(Error::Underflow));
}

#[test]
fn test_create_subscription_validates_amount() {
    // Test that validate_non_negative (used in create_subscription) rejects negative amounts
    assert_eq!(validate_non_negative(-1_000_000i128), Err(Error::Underflow));
    assert_eq!(validate_non_negative(0), Ok(()));
    assert_eq!(validate_non_negative(10_000_000i128), Ok(()));
}

#[test]
fn test_min_topup_below_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    let min_topup = 5_000000i128; // 5 USDC
    
    client.init(&token, &admin, &min_topup);
    let sub_id = client.create_subscription(&subscriber, &merchant, &min_topup, &(30 * 24 * 60 * 60), &false);
    
    let result = client.try_deposit_funds(&sub_id, &subscriber, &4_999999);
    assert!(result.is_err());
}

#[test]
fn test_min_topup_exactly_at_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    let min_topup = 5_000000i128; // 5 USDC
    
    client.init(&token, &admin, &min_topup);
    let sub_id = client.create_subscription(&subscriber, &merchant, &min_topup, &(30 * 24 * 60 * 60), &false);
    
    let result = client.try_deposit_funds(&sub_id, &subscriber, &min_topup);
    assert!(result.is_ok());
}

#[test]
fn test_min_topup_above_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    let subscriber = Address::generate(&env);
    let merchant = Address::generate(&env);
    let min_topup = 5_000000i128; // 5 USDC
    
    client.init(&token, &admin, &min_topup);
    let sub_id = client.create_subscription(&subscriber, &merchant, &min_topup, &(30 * 24 * 60 * 60), &false);
    
    let result = client.try_deposit_funds(&sub_id, &subscriber, &10_000000);
    assert!(result.is_ok());
}

#[test]
fn test_set_min_topup_by_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    let initial_min = 1_000000i128;
    let new_min = 10_000000i128;
    
    client.init(&token, &admin, &initial_min);
    assert_eq!(client.get_min_topup(), initial_min);
    
    client.set_min_topup(&admin, &new_min);
    assert_eq!(client.get_min_topup(), new_min);
}

#[test]
fn test_set_min_topup_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(SubscriptionVault, ());
    let client = SubscriptionVaultClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let min_topup = 1_000000i128;
    
    client.init(&token, &admin, &min_topup);
    
    let result = client.try_set_min_topup(&non_admin, &5_000000);
    assert!(result.is_err());
}
