//! CryptoTrove Legacy Vault
//!
//! A dead-man's-switch for next of kin.
//!
//!   Active   -> owner checked in within the check-in period (default 12 months)
//!   Grace    -> check-in missed; a second countdown (default 90 days) is running.
//!               The owner can still cancel by calling `check_in`.
//!   Released -> grace period also expired. The vault is final and the
//!               beneficiary can claim it.
//!
//! IMPORTANT: nothing secret is stored here. Everything on-chain is public.
//! The contract only stores a pointer to an *encrypted* recovery package
//! (encrypted in the owner's browser, unlocked by a passphrase the owner gives
//! the beneficiary offline) and a hash of the beneficiary's email. The seed
//! phrase itself never touches the chain.

use near_sdk::store::LookupMap;
use near_sdk::{env, near, require, AccountId, PanicOnDefault};

/// Minimum allowed period. Low on purpose so the demo can run in minutes.
const MIN_PERIOD_SECS: u64 = 60;
const MAX_POINTER_LEN: usize = 256;
const MAX_HASH_LEN: usize = 128;

#[near(serializers = [json])]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Active,
    Grace,
    Released,
}

#[near(serializers = [borsh])]
pub struct Vault {
    pub last_check_in_secs: u64,
    pub check_in_period_secs: u64,
    pub grace_period_secs: u64,
    /// Pointer (e.g. an IPFS CID) to the ENCRYPTED recovery package.
    pub encrypted_pointer: String,
    /// Hash of the beneficiary's email. The real email is kept off-chain.
    pub beneficiary_hash: String,
}

#[near(serializers = [json])]
pub struct VaultView {
    pub owner: AccountId,
    pub status: Status,
    pub last_check_in_secs: u64,
    pub check_in_period_secs: u64,
    pub grace_period_secs: u64,
    /// When the active period ends and the grace period starts.
    pub grace_starts_at_secs: u64,
    /// When the grace period ends and the vault is released.
    pub release_at_secs: u64,
    pub beneficiary_hash: String,
}

fn now_secs() -> u64 {
    env::block_timestamp() / 1_000_000_000
}

fn status_of(vault: &Vault, now: u64) -> Status {
    let grace_starts = vault.last_check_in_secs + vault.check_in_period_secs;
    let release_at = grace_starts + vault.grace_period_secs;
    if now < grace_starts {
        Status::Active
    } else if now < release_at {
        Status::Grace
    } else {
        Status::Released
    }
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    vaults: LookupMap<AccountId, Vault>,
}

#[near]
impl Contract {
    #[init]
    pub fn new() -> Self {
        Self {
            vaults: LookupMap::new(b"v".to_vec()),
        }
    }

    // ---------- owner actions ----------

    /// Create your vault. Periods are in seconds
    /// (12 months = 31_536_000, 90 days = 7_776_000).
    pub fn create_vault(
        &mut self,
        check_in_period_secs: u64,
        grace_period_secs: u64,
        encrypted_pointer: String,
        beneficiary_hash: String,
    ) {
        let owner = env::predecessor_account_id();
        require!(!self.vaults.contains_key(&owner), "Vault already exists");
        require!(
            check_in_period_secs >= MIN_PERIOD_SECS && grace_period_secs >= MIN_PERIOD_SECS,
            "Periods must be at least 60 seconds"
        );
        require!(
            encrypted_pointer.len() <= MAX_POINTER_LEN && beneficiary_hash.len() <= MAX_HASH_LEN,
            "Pointer or hash too long"
        );
        self.vaults.insert(
            owner,
            Vault {
                last_check_in_secs: now_secs(),
                check_in_period_secs,
                grace_period_secs,
                encrypted_pointer,
                beneficiary_hash,
            },
        );
    }

    /// Prove you are alive. Resets the countdown, and during the grace period
    /// this is also how you cancel the transfer. Not possible once Released.
    pub fn check_in(&mut self) {
        let owner = env::predecessor_account_id();
        let now = now_secs();
        let vault = self
            .vaults
            .get_mut(&owner)
            .unwrap_or_else(|| env::panic_str("No vault for this account"));
        require!(
            status_of(vault, now) != Status::Released,
            "Vault already released"
        );
        vault.last_check_in_secs = now;
    }

    /// Change the encrypted package pointer and/or beneficiary hash.
    /// Also counts as a check-in.
    pub fn update_vault(
        &mut self,
        encrypted_pointer: Option<String>,
        beneficiary_hash: Option<String>,
    ) {
        let owner = env::predecessor_account_id();
        let now = now_secs();
        let vault = self
            .vaults
            .get_mut(&owner)
            .unwrap_or_else(|| env::panic_str("No vault for this account"));
        require!(
            status_of(vault, now) != Status::Released,
            "Vault already released"
        );
        if let Some(p) = encrypted_pointer {
            require!(p.len() <= MAX_POINTER_LEN, "Pointer too long");
            vault.encrypted_pointer = p;
        }
        if let Some(h) = beneficiary_hash {
            require!(h.len() <= MAX_HASH_LEN, "Hash too long");
            vault.beneficiary_hash = h;
        }
        vault.last_check_in_secs = now;
    }

    /// Delete your vault entirely (only before it is released).
    pub fn delete_vault(&mut self) {
        let owner = env::predecessor_account_id();
        let now = now_secs();
        let vault = self
            .vaults
            .get(&owner)
            .unwrap_or_else(|| env::panic_str("No vault for this account"));
        require!(
            status_of(vault, now) != Status::Released,
            "Vault already released"
        );
        self.vaults.remove(&owner);
    }

    // ---------- views (free to call) ----------

    pub fn get_status(&self, owner: AccountId) -> Option<Status> {
        self.vaults.get(&owner).map(|v| status_of(v, now_secs()))
    }

    pub fn get_vault(&self, owner: AccountId) -> Option<VaultView> {
        let now = now_secs();
        self.vaults.get(&owner).map(|v| {
            let grace_starts = v.last_check_in_secs + v.check_in_period_secs;
            VaultView {
                owner: owner.clone(),
                status: status_of(v, now),
                last_check_in_secs: v.last_check_in_secs,
                check_in_period_secs: v.check_in_period_secs,
                grace_period_secs: v.grace_period_secs,
                grace_starts_at_secs: grace_starts,
                release_at_secs: grace_starts + v.grace_period_secs,
                beneficiary_hash: v.beneficiary_hash.clone(),
            }
        })
    }

    /// Returns the pointer to the encrypted package ONLY once the vault is
    /// Released. Used by the claim page. (The chain is public, so the real
    /// protection is the passphrase encryption, not this check.)
    pub fn get_released_package(&self, owner: AccountId) -> Option<String> {
        let now = now_secs();
        self.vaults.get(&owner).and_then(|v| {
            if status_of(v, now) == Status::Released {
                Some(v.encrypted_pointer.clone())
            } else {
                None
            }
        })
    }
}
