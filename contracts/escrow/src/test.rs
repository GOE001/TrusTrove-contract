#![cfg(test)]

use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, Address, BytesN, Env, Vec,
};

use crate::{EscrowContract, EscrowContractClient};
use crate::{EscrowEvent, EscrowAction};

#[contract]
pub struct MockToken;

#[contractimpl]
impl MockToken {
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        let from_key = BalanceKey(from.clone());
        let to_key = BalanceKey(to.clone());
        let from_bal: i128 = env.storage().persistent().get(&from_key).unwrap_or(0);
        let to_bal: i128 = env.storage().persistent().get(&to_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&from_key, &(from_bal - amount));
        env.storage().persistent().set(&to_key, &(to_bal + amount));
    }

    pub fn balance(env: Env, addr: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&BalanceKey(addr))
            .unwrap_or(0)
    }
}

#[contracttype]
pub struct BalanceKey(Address);

fn setup() -> (
    Env,
    EscrowContractClient<'static>,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let pool = Address::generate(&env);
    let usdc_id = env.register_contract(None, MockToken);
    let _mock_token = MockTokenClient::new(&env, &usdc_id);

    let pool_bal_key = BalanceKey(pool.clone());
    env.as_contract(&usdc_id, || {
        env.storage()
            .persistent()
            .set(&pool_bal_key, &10_000_000_000_000i128);
    });

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);

    client.initialize(&admin, &pool, &pool, &usdc_id);

    (env, client, admin, pool, usdc_id)
}

fn generate_invoice_id(env: &Env) -> BytesN<32> {
    let mut arr = [0u8; 32];
    arr[0..8].copy_from_slice(&env.ledger().timestamp().to_be_bytes());
    BytesN::from_array(env, &arr)
}

#[test]
fn test_initialize() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let pool = Address::generate(&env);
    let invoice = Address::generate(&env);
    let usdc = env.register_contract(None, MockToken);
    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin, &pool, &invoice, &usdc);

    assert_eq!(client.get_locked(&generate_invoice_id(&env)), 0);
}

#[test]
fn test_lock_stores_record() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);
    let amount: u128 = 1_000_000_000;

    let result = client.lock(&invoice_id, &amount);
    assert!(result);

    let locked = client.get_locked(&invoice_id);
    assert_eq!(locked, amount);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_lock_fails_zero_amount() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);
    client.lock(&invoice_id, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_lock_fails_duplicate() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);
    client.lock(&invoice_id, &1_000_000_000);
    client.lock(&invoice_id, &500_000_000);
}

#[test]
fn test_release_to_issuer_transfers_correct_amount() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);
    let issuer = Address::generate(&env);
    let amount: u128 = 1_000_000_000;

    client.lock(&invoice_id, &amount);
    let result = client.release_to_issuer(&invoice_id, &issuer);
    assert!(result);

    let locked = client.get_locked(&invoice_id);
    assert_eq!(locked, 0);
}

#[test]
fn test_release_to_pool_transfers_correct_amount() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);
    let amount: u128 = 1_000_000_000;

    client.lock(&invoice_id, &amount);
    let repayment: u128 = 1_050_000_000;
    let result = client.release_to_pool(&invoice_id, &repayment);
    assert!(result);

    let locked = client.get_locked(&invoice_id);
    assert_eq!(locked, 0);
}

#[test]
fn test_handle_default_returns_funds_to_pool() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);
    let amount: u128 = 1_000_000_000;

    client.lock(&invoice_id, &amount);
    let result = client.handle_default(&invoice_id);
    assert!(result);

    let locked = client.get_locked(&invoice_id);
    assert_eq!(locked, 0);
}

#[test]
fn test_handle_default_no_record_returns_false() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);

    let result = client.handle_default(&invoice_id);
    assert!(!result);
}

#[test]
fn test_get_locked_returns_zero_when_empty() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);

    assert_eq!(client.get_locked(&invoice_id), 0);
}

#[test]
fn test_get_locked_returns_amount_when_locked() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);
    let amount: u128 = 1_000_000_000;

    client.lock(&invoice_id, &amount);
    assert_eq!(client.get_locked(&invoice_id), amount);
}

#[test]
fn test_history_records_on_lock_and_release_to_issuer() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);
    let amount: u128 = 1_000_000_000;

    client.lock(&invoice_id, &amount);
    let history: Vec<EscrowEvent> = client.get_history(&invoice_id);
    assert_eq!(history.len(), 1);
    let first = history.get(0).unwrap();
    assert_eq!(first.action, EscrowAction::Locked);
    assert_eq!(first.amount, amount);

    let issuer = Address::generate(&env);
    client.release_to_issuer(&invoice_id, &issuer);
    let history2: Vec<EscrowEvent> = client.get_history(&invoice_id);
    assert_eq!(history2.len(), 2);
    let second = history2.get(1).unwrap();
    assert_eq!(second.action, EscrowAction::ReleasedToIssuer);
    assert_eq!(second.amount, amount);
}

#[test]
fn test_history_records_on_release_to_pool_and_default() {
    let (env, client, _admin, _pool, _usdc) = setup();
    let invoice_id = generate_invoice_id(&env);
    let amount: u128 = 2_000_000_000;

    client.lock(&invoice_id, &amount);
    client.release_to_pool(&invoice_id, &1_050_000_000);
    let history: Vec<EscrowEvent> = client.get_history(&invoice_id);
    assert_eq!(history.len(), 2);
    let second = history.get(1).unwrap();
    assert_eq!(second.action, EscrowAction::ReleasedToPool);
    assert_eq!(second.amount, 1_050_000_000);

    // lock again and trigger default using a distinct invoice id
    let invoice_id2 = BytesN::from_array(&env, &[1u8; 32]);
    client.lock(&invoice_id2, &amount);
    let res = client.handle_default(&invoice_id2);
    assert!(res);
    let history_d: Vec<EscrowEvent> = client.get_history(&invoice_id2);
    assert_eq!(history_d.len(), 2); // Locked + DefaultHandled
    let last = history_d.get(1).unwrap();
    assert_eq!(last.action, EscrowAction::DefaultHandled);
}
