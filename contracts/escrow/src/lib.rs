#![no_std]

use soroban_sdk::{contract, contractimpl, panic_with_error, token, Address, BytesN, Env, Vec};

mod errors;
mod test;
mod types;

pub use errors::*;
pub use types::*;

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        pool_contract: Address,
        invoice_contract: Address,
        usdc_asset: Address,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic_with_error!(&env, EscrowError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::PoolContract, &pool_contract);
        env.storage()
            .instance()
            .set(&DataKey::InvoiceContract, &invoice_contract);
        env.storage()
            .instance()
            .set(&DataKey::UsdcAsset, &usdc_asset);
    }

    pub fn lock(env: Env, invoice_id: BytesN<32>, amount: u128) -> bool {
        let pool: Address = env
            .storage()
            .instance()
            .get(&DataKey::PoolContract)
            .unwrap();
        pool.require_auth();

        if amount == 0 {
            panic_with_error!(&env, EscrowError::InvalidAmount);
        }

        let key = DataKey::Locked(invoice_id.clone());
        if env.storage().persistent().has(&key) {
            panic_with_error!(&env, EscrowError::AlreadyLocked);
        }

        let usdc_id: Address = env.storage().instance().get(&DataKey::UsdcAsset).unwrap();
        let usdc = token::Client::new(&env, &usdc_id);
        usdc.transfer(&pool, &env.current_contract_address(), &(amount as i128));

        let record = EscrowRecord {
            invoice_id: invoice_id.clone(),
            amount,
            locked_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&key, &record);
        env.storage().persistent().extend_ttl(&key, 100, 2_000_000);

        // Append history event
        let event = EscrowEvent {
            invoice_id: invoice_id.clone(),
            action: EscrowAction::Locked,
            amount,
            timestamp: env.ledger().timestamp(),
        };
        append_history(&env, &invoice_id, event);

        true
    }

    pub fn release_to_issuer(env: Env, invoice_id: BytesN<32>, issuer: Address) -> bool {
        let pool: Address = env
            .storage()
            .instance()
            .get(&DataKey::PoolContract)
            .unwrap();
        pool.require_auth();

        let key = DataKey::Locked(invoice_id.clone());
        let record: EscrowRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, EscrowError::NotFound));

        let usdc_id: Address = env.storage().instance().get(&DataKey::UsdcAsset).unwrap();
        let usdc = token::Client::new(&env, &usdc_id);
        usdc.transfer(
            &env.current_contract_address(),
            &issuer,
            &(record.amount as i128),
        );
        // append history event for release to issuer
        let event = EscrowEvent {
            invoice_id: invoice_id.clone(),
            action: EscrowAction::ReleasedToIssuer,
            amount: record.amount,
            timestamp: env.ledger().timestamp(),
        };
        append_history(&env, &invoice_id, event);

        env.storage().persistent().remove(&key);
        true
    }

    pub fn release_to_pool(env: Env, invoice_id: BytesN<32>, repayment_amount: u128) -> bool {
        let pool: Address = env
            .storage()
            .instance()
            .get(&DataKey::PoolContract)
            .unwrap();
        pool.require_auth();

        let key = DataKey::Locked(invoice_id.clone());
        let _record: EscrowRecord = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, EscrowError::NotFound));

        let usdc_id: Address = env.storage().instance().get(&DataKey::UsdcAsset).unwrap();
        let usdc = token::Client::new(&env, &usdc_id);
        usdc.transfer(
            &env.current_contract_address(),
            &pool,
            &(repayment_amount as i128),
        );
        // append history event for release to pool
        let event = EscrowEvent {
            invoice_id: invoice_id.clone(),
            action: EscrowAction::ReleasedToPool,
            amount: repayment_amount,
            timestamp: env.ledger().timestamp(),
        };
        append_history(&env, &invoice_id, event);

        env.storage().persistent().remove(&key);
        true
    }

    pub fn handle_default(env: Env, invoice_id: BytesN<32>) -> bool {
        let key = DataKey::Locked(invoice_id.clone());
        if !env.storage().persistent().has(&key) {
            return false;
        }

        let _admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        let pool: Address = env
            .storage()
            .instance()
            .get(&DataKey::PoolContract)
            .unwrap();

        // Try admin auth first; if that's not the caller, try pool
        // In Soroban, we can't easily catch require_auth failures,
        // so we require the pool to auth (the common case)
        pool.require_auth();

        let record: EscrowRecord = env.storage().persistent().get(&key).unwrap();
        let usdc_id: Address = env.storage().instance().get(&DataKey::UsdcAsset).unwrap();
        let usdc = token::Client::new(&env, &usdc_id);
        usdc.transfer(
            &env.current_contract_address(),
            &pool,
            &(record.amount as i128),
        );
        // append history event for default handled
        let event = EscrowEvent {
            invoice_id: invoice_id.clone(),
            action: EscrowAction::DefaultHandled,
            amount: record.amount,
            timestamp: env.ledger().timestamp(),
        };
        append_history(&env, &invoice_id, event);

        env.storage().persistent().remove(&key);
        true
    }

    pub fn get_locked(env: Env, invoice_id: BytesN<32>) -> u128 {
        env.storage()
            .persistent()
            .get::<_, EscrowRecord>(&DataKey::Locked(invoice_id))
            .map(|r| r.amount)
            .unwrap_or(0)
    }

    pub fn get_history(env: Env, invoice_id: BytesN<32>) -> Vec<EscrowEvent> {
        env.storage()
            .persistent()
            .get(&DataKey::History(invoice_id))
            .unwrap_or(Vec::new(&env))
    }
}

fn append_history(env: &Env, invoice_id: &BytesN<32>, event: EscrowEvent) {
    let key = DataKey::History(invoice_id.clone());
    let mut history: Vec<EscrowEvent> = env.storage().persistent().get(&key).unwrap_or(Vec::new(env));
    history.push_back(event);
    env.storage().persistent().set(&key, &history);
    env.storage().persistent().extend_ttl(&key, 100, 2_000_000);
}
