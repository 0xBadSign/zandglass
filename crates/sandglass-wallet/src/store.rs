//! The on-disk store: `~/.sandglass/<network>/`.
//!
//! ```text
//! <root>/
//!   keycheck.age          passphrase check (age scrypt)
//!   plans.json            plans and vault epochs (no secrets)
//!   vaults/<plan>/e<N>/
//!     seed.age            the vault seed, age passphrase (scrypt) encrypted
//!     wallet.sqlite       the vault's light-client wallet (viewing keys only)
//! ```
//!
//! A vault seed is never written to disk in plaintext.

use std::fs;
use std::io::{Read, Write};
use std::iter;
use std::path::{Path, PathBuf};

use age::secrecy::SecretString;
use age::{scrypt, Decryptor, Encryptor};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::fsutil::{create_private_dir, write_private_atomic as write_private};

/// The plan file format version.
pub const PLANS_VERSION: u32 = 1;
/// Default scrypt work factor (log2 N) for seed files.
pub const DEFAULT_WORK_FACTOR: u8 = 18;
/// Largest scrypt work factor accepted when reading our own files.
const MAX_WORK_FACTOR: u8 = 22;
const KEYCHECK_FILE: &str = "keycheck.age";
const KEYCHECK_TEXT: &str = "sandglass-store-v1";
const PLANS_FILE: &str = "plans.json";
const SEED_FILE: &str = "seed.age";

/// Store errors that callers branch on.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The store directory has not been initialised.
    #[error("no store found at {0}; run `sandglass init` first")]
    NotInitialised(String),
    /// The store directory already exists.
    #[error("a store already exists at {0}")]
    AlreadyInitialised(String),
    /// The passphrase does not open the store.
    #[error("wrong passphrase")]
    WrongPassphrase,
    /// No plan matches.
    #[error("no plan matches `{0}`")]
    PlanNotFound(String),
    /// The prefix matches several plans.
    #[error("`{0}` matches more than one plan; use a longer id")]
    PlanAmbiguous(String),
}

/// Whether a plan is live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    /// Funds are sealed and the owner flips to keep them.
    Active,
    /// The owner swept the funds back.
    Cancelled,
}

/// One vault epoch of a plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRecord {
    /// Epoch number, from 1.
    pub epoch: u32,
    /// Vault directory, relative to the store root.
    pub dir: String,
    /// Wallet birthday height.
    pub birthday_height: u32,
    /// drand round the kit unlocks at.
    pub unlock_round: u64,
    /// Unix time of that round.
    pub unlock_time: u64,
    /// Unix creation time.
    pub created: u64,
}

/// A plan: heirs, period and the vault epochs created so far.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Random 128-bit id, lowercase hex.
    pub id: String,
    /// `legacy` (installments come in M2).
    pub kind: String,
    /// `main` or `test`.
    pub network: String,
    /// Heir codes (public).
    pub heirs: Vec<String>,
    /// Seconds between flips, and the lock length of each kit.
    pub period_secs: u64,
    /// Optional message sealed into every kit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The current epoch.
    pub epoch: u32,
    /// Active or cancelled.
    pub status: PlanStatus,
    /// Unix creation time.
    pub created: u64,
    /// Optional keeper mailbox URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox_url: Option<String>,
    /// Mailbox id on that keeper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox_id: Option<String>,
    /// Token that may replace or delete the kit in the mailbox. It opens no
    /// funds and no kit, so it is stored with the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox_write_token: Option<String>,
    /// All epochs, oldest first. For an installment plan, epoch 1 is the
    /// funding vault.
    pub epochs: Vec<EpochRecord>,
    /// Installment schedule (installment plans only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installments: Option<InstallmentSet>,
    /// A new epoch whose vault a flip has swept to (or tried to) but that
    /// is not yet confirmed to hold the funds. It is never deleted or reused
    /// until its balance has been checked: see `ops::adopt_pending`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<EpochRecord>,
}

/// One installment vault of an installment plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallmentRecord {
    /// 1-based position.
    pub index: u32,
    /// Vault directory, relative to the store root.
    pub dir: String,
    /// Wallet birthday height.
    pub birthday_height: u32,
    /// drand round the kit unlocks at.
    pub unlock_round: u64,
    /// Unix time of that round.
    pub unlock_time: u64,
    /// Kit file name that was written.
    pub kit_file: String,
    /// Keeper mailbox id of this installment's kit, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox_id: Option<String>,
    /// Write token of that mailbox (it opens no funds and no kit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox_write_token: Option<String>,
}

/// The schedule of an installment plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallmentSet {
    /// What the payee receives per installment, in zatoshi.
    pub amount_zat: u64,
    /// Extra zatoshi in each vault so the payee's sweep fee is covered.
    pub fee_zat: u64,
    /// Seconds between installments.
    pub every_secs: u64,
    /// Number of installments.
    pub count: u32,
    /// Absolute start (unix time) when the owner gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at: Option<u64>,
    /// Otherwise the first installment unlocks this long after the split.
    pub start_after_secs: u64,
    /// Directory for the kit files and the claim-links file (`--out`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_dir: Option<String>,
    /// Transaction id of the split, once done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_txid: Option<String>,
    /// The installment vaults (empty until the split).
    #[serde(default)]
    pub items: Vec<InstallmentRecord>,
}

impl InstallmentSet {
    /// Zatoshi each installment vault holds.
    pub fn vault_zat(&self) -> u64 {
        self.amount_zat + self.fee_zat
    }
}

impl Plan {
    /// The first eight characters of the id, for file names and display.
    pub fn short_id(&self) -> &str {
        &self.id[..8]
    }

    /// The current epoch record.
    pub fn current(&self) -> Result<&EpochRecord> {
        self.epochs
            .iter()
            .find(|e| e.epoch == self.epoch)
            .ok_or_else(|| anyhow!("plan {} has no epoch {}", self.short_id(), self.epoch))
    }
}

#[derive(Serialize, Deserialize)]
struct PlansFile {
    version: u32,
    plans: Vec<Plan>,
}

/// An opened store (passphrase verified).
pub struct Store {
    root: PathBuf,
    passphrase: SecretString,
    work_factor: u8,
}

impl Store {
    /// Creates a new store in `root` protected by `passphrase`.
    pub fn init(root: &Path, passphrase: &str) -> Result<Self> {
        Self::init_with(root, passphrase, DEFAULT_WORK_FACTOR)
    }

    /// Like [`Store::init`] with an explicit scrypt work factor (tests use a low one).
    pub fn init_with(root: &Path, passphrase: &str, work_factor: u8) -> Result<Self> {
        if root.join(KEYCHECK_FILE).exists() {
            return Err(StoreError::AlreadyInitialised(root.display().to_string()).into());
        }
        if passphrase.is_empty() {
            bail!("the passphrase must not be empty");
        }
        create_private_dir(root)?;
        let store = Self {
            root: root.to_path_buf(),
            passphrase: SecretString::from(passphrase.to_string()),
            work_factor,
        };
        let check = store.encrypt(KEYCHECK_TEXT.as_bytes())?;
        write_private(&root.join(KEYCHECK_FILE), &check)?;
        store.save_plans(&[])?;
        Ok(store)
    }

    /// Opens an existing store and verifies the passphrase.
    pub fn open(root: &Path, passphrase: &str) -> Result<Self> {
        let path = root.join(KEYCHECK_FILE);
        if !path.exists() {
            return Err(StoreError::NotInitialised(root.display().to_string()).into());
        }
        let store = Self {
            root: root.to_path_buf(),
            passphrase: SecretString::from(passphrase.to_string()),
            work_factor: DEFAULT_WORK_FACTOR,
        };
        let data = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let text = store.decrypt(&data)?;
        if text.as_slice() != KEYCHECK_TEXT.as_bytes() {
            return Err(StoreError::WrongPassphrase.into());
        }
        Ok(store)
    }

    /// The store root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All plans.
    pub fn plans(&self) -> Result<Vec<Plan>> {
        let path = self.root.join(PLANS_FILE);
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let file: PlansFile = serde_json::from_str(&text).context("parsing plans.json")?;
        if file.version != PLANS_VERSION {
            bail!("plans.json has unsupported version {}", file.version);
        }
        Ok(file.plans)
    }

    /// Replaces the plan list atomically.
    pub fn save_plans(&self, plans: &[Plan]) -> Result<()> {
        let file = PlansFile {
            version: PLANS_VERSION,
            plans: plans.to_vec(),
        };
        let text = serde_json::to_string_pretty(&file)?;
        write_private(&self.root.join(PLANS_FILE), text.as_bytes())
    }

    /// Adds or replaces (by id) one plan.
    pub fn put_plan(&self, plan: &Plan) -> Result<()> {
        let mut plans = self.plans()?;
        match plans.iter_mut().find(|p| p.id == plan.id) {
            Some(slot) => *slot = plan.clone(),
            None => plans.push(plan.clone()),
        }
        self.save_plans(&plans)
    }

    /// Finds a plan by id prefix. With `None`, returns the only active plan.
    pub fn find_plan(&self, prefix: Option<&str>) -> Result<Plan> {
        let plans = self.plans()?;
        match prefix {
            Some(p) => {
                let mut hits = plans.into_iter().filter(|plan| plan.id.starts_with(p));
                let first = hits
                    .next()
                    .ok_or_else(|| StoreError::PlanNotFound(p.into()))?;
                if hits.next().is_some() {
                    return Err(StoreError::PlanAmbiguous(p.into()).into());
                }
                Ok(first)
            }
            None => {
                let mut active = plans.into_iter().filter(|p| p.status == PlanStatus::Active);
                let first = active
                    .next()
                    .ok_or_else(|| StoreError::PlanNotFound("(no active plan)".into()))?;
                if active.next().is_some() {
                    bail!("several plans are active; name one (see `sandglass status`)");
                }
                Ok(first)
            }
        }
    }

    /// The vault directory (relative to the root) of an installment vault.
    pub fn installment_rel_dir(plan_id: &str, index: u32) -> String {
        format!("vaults/{plan_id}/i{index}")
    }

    /// The vault directory (relative to the root) of an epoch.
    pub fn vault_rel_dir(plan_id: &str, epoch: u32) -> String {
        format!("vaults/{plan_id}/e{epoch}")
    }

    /// Absolute path of a relative store path.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Encrypts and writes a vault seed phrase to `<dir>/seed.age`.
    pub fn write_seed(&self, rel_dir: &str, phrase: &str) -> Result<()> {
        let dir = self.path(rel_dir);
        create_private_dir(&dir)?;
        let file = dir.join(SEED_FILE);
        if file.exists() {
            bail!("{} already exists", file.display());
        }
        write_private(&file, &self.encrypt(phrase.as_bytes())?)
    }

    /// Reads and decrypts a vault seed phrase.
    pub fn read_seed(&self, rel_dir: &str) -> Result<Zeroizing<String>> {
        let file = self.path(rel_dir).join(SEED_FILE);
        let data = fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
        let plain = self.decrypt(&data)?;
        let text =
            String::from_utf8(plain.to_vec()).map_err(|_| anyhow!("seed file is damaged"))?;
        Ok(Zeroizing::new(text))
    }

    fn encrypt(&self, plain: &[u8]) -> Result<Vec<u8>> {
        let mut recipient = scrypt::Recipient::new(self.passphrase.clone());
        recipient.set_work_factor(self.work_factor);
        let encryptor = Encryptor::with_recipients(iter::once(&recipient as _))
            .map_err(|e| anyhow!("encryption setup failed: {e}"))?;
        let mut out = Vec::new();
        let mut w = encryptor
            .wrap_output(&mut out)
            .map_err(|e| anyhow!("encryption failed: {e}"))?;
        w.write_all(plain)?;
        w.finish()?;
        Ok(out)
    }

    fn decrypt(&self, data: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        let decryptor = Decryptor::new(data).map_err(|e| anyhow!("file is damaged: {e}"))?;
        let mut identity = scrypt::Identity::new(self.passphrase.clone());
        // The default bound is benchmarked per run and can be too low under load.
        identity.set_max_work_factor(MAX_WORK_FACTOR);
        let mut reader = decryptor
            .decrypt(iter::once(&identity as _))
            .map_err(|e| match e {
                age::DecryptError::DecryptionFailed | age::DecryptError::NoMatchingKeys => {
                    anyhow::Error::new(StoreError::WrongPassphrase)
                }
                other => anyhow!("decryption failed: {other}"),
            })?;
        let mut out = Zeroizing::new(Vec::new());
        reader.read_to_end(&mut out)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title";

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn plan(id: &str) -> Plan {
        Plan {
            id: id.into(),
            kind: "legacy".into(),
            network: "test".into(),
            heirs: vec!["age1x".into()],
            period_secs: 180,
            message: None,
            epoch: 1,
            status: PlanStatus::Active,
            created: 1,
            mailbox_url: None,
            mailbox_id: None,
            mailbox_write_token: None,
            epochs: vec![EpochRecord {
                epoch: 1,
                dir: Store::vault_rel_dir(id, 1),
                birthday_height: 10,
                unlock_round: 99,
                unlock_time: 100,
                created: 1,
            }],
            installments: None,
            pending: None,
        }
    }

    /// Every file under `dir`, contents included.
    fn all_files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        for entry in fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                out.extend(all_files(&p));
            } else {
                out.push((p.clone(), fs::read(&p).unwrap()));
            }
        }
        out
    }

    #[test]
    fn seed_is_encrypted_and_round_trips() {
        let dir = tmp();
        let root = dir.path().join("test");
        let store = Store::init_with(&root, "correct horse", 10).unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        let rel = Store::vault_rel_dir(id, 1);
        store.write_seed(&rel, SEED).unwrap();
        store.put_plan(&plan(id)).unwrap();

        // No file contains any seed word sequence, plaintext or as raw bytes.
        let files = all_files(&root);
        assert!(files.len() >= 3);
        for (path, data) in &files {
            let text = String::from_utf8_lossy(data);
            for needle in [SEED, "legal winner", "sausage worth"] {
                assert!(
                    !text.contains(needle),
                    "{} leaks seed words",
                    path.display()
                );
            }
        }
        let seed_file = files
            .iter()
            .find(|(p, _)| p.ends_with("seed.age"))
            .expect("seed.age exists");
        assert!(seed_file.1.starts_with(b"age-encryption.org/v1"));
        assert!(String::from_utf8_lossy(&seed_file.1).contains("scrypt"));

        let reopened = Store::open(&root, "correct horse").unwrap();
        assert_eq!(reopened.read_seed(&rel).unwrap().as_str(), SEED);
        assert_eq!(reopened.find_plan(Some("0123")).unwrap().id, id);
        assert_eq!(reopened.find_plan(None).unwrap().id, id);
    }

    #[test]
    fn wrong_passphrase_is_refused() {
        let dir = tmp();
        let root = dir.path().join("s");
        Store::init_with(&root, "right", 10).unwrap();
        let err = Store::open(&root, "wrong").err().unwrap();
        assert!(matches!(
            err.downcast_ref::<StoreError>(),
            Some(StoreError::WrongPassphrase)
        ));
    }

    #[test]
    fn init_twice_and_missing_store() {
        let dir = tmp();
        let root = dir.path().join("s");
        assert!(matches!(
            Store::open(&root, "x").err().unwrap().downcast_ref(),
            Some(StoreError::NotInitialised(_))
        ));
        Store::init_with(&root, "x", 10).unwrap();
        assert!(matches!(
            Store::init_with(&root, "x", 10)
                .err()
                .unwrap()
                .downcast_ref(),
            Some(StoreError::AlreadyInitialised(_))
        ));
    }

    #[test]
    fn plan_lookup_rules() {
        let dir = tmp();
        let store = Store::init_with(&dir.path().join("s"), "x", 10).unwrap();
        let a = "aaaa0000000000000000000000000001";
        let b = "aaaa0000000000000000000000000002";
        store.put_plan(&plan(a)).unwrap();
        store.put_plan(&plan(b)).unwrap();
        assert!(store.find_plan(Some("aaaa")).is_err());
        assert_eq!(
            store
                .find_plan(Some("aaaa0000000000000000000000000002"))
                .unwrap()
                .id,
            b
        );
        assert!(store.find_plan(None).is_err());
        assert!(store.find_plan(Some("ffff")).is_err());
    }
}
