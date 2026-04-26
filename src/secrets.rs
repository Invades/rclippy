use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

use anyhow::{Context, Result};
use rand::{RngCore, rngs::OsRng};
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};

use crate::APP_NAME;

const VAULT_KEY: &str = "vault-v1";

static KEYCHAIN_CACHE: LazyLock<Mutex<HashMap<String, Option<Vec<u8>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub device_id: String,
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
}

impl Identity {
    pub fn cert(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert_der.clone())
    }

    pub fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivatePkcs8KeyDer::from(self.key_der.clone()).into()
    }

    pub fn cert_fingerprint(&self) -> String {
        cert_fingerprint(&self.cert_der)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerIdentity {
    pub device_id: String,
    pub device_name: String,
    pub cert_der: Vec<u8>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct StoredIdentity {
    device_id: String,
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
}

impl From<Identity> for StoredIdentity {
    fn from(identity: Identity) -> Self {
        Self {
            device_id: identity.device_id,
            cert_der: identity.cert_der,
            key_der: identity.key_der,
        }
    }
}

impl From<StoredIdentity> for Identity {
    fn from(stored: StoredIdentity) -> Self {
        Self {
            device_id: stored.device_id,
            cert_der: stored.cert_der,
            key_der: stored.key_der,
        }
    }
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct StoredPeerIdentity {
    device_id: String,
    device_name: String,
    cert_der: Vec<u8>,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
struct StoredVault {
    identity: Option<StoredIdentity>,
    peer: Option<StoredPeerIdentity>,
}

impl From<PeerIdentity> for StoredPeerIdentity {
    fn from(peer: PeerIdentity) -> Self {
        Self {
            device_id: peer.device_id,
            device_name: peer.device_name,
            cert_der: peer.cert_der,
        }
    }
}

impl From<StoredPeerIdentity> for PeerIdentity {
    fn from(stored: StoredPeerIdentity) -> Self {
        Self {
            device_id: stored.device_id,
            device_name: stored.device_name,
            cert_der: stored.cert_der,
        }
    }
}

impl PeerIdentity {
    pub fn display_name(&self) -> &str {
        if self.device_name.trim().is_empty() {
            &self.device_id
        } else {
            &self.device_name
        }
    }

    pub fn cert(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert_der.clone())
    }

    pub fn cert_fingerprint(&self) -> String {
        cert_fingerprint(&self.cert_der)
    }
}

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn set(&self, key: &str, value: &[u8]) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct KeychainSecretStore;

impl SecretStore for KeychainSecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        if let Some(cached) = KEYCHAIN_CACHE
            .lock()
            .expect("keychain cache poisoned")
            .get(key)
            .cloned()
        {
            return Ok(cached);
        }

        let entry = keyring::Entry::new(APP_NAME, key)?;
        let value = match entry.get_secret() {
            Ok(secret) => Some(secret),
            Err(keyring::Error::NoEntry) => None,
            Err(err) => return Err(err.into()),
        };
        KEYCHAIN_CACHE
            .lock()
            .expect("keychain cache poisoned")
            .insert(key.to_owned(), value.clone());
        Ok(value)
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        let entry = keyring::Entry::new(APP_NAME, key)?;
        entry.set_secret(value)?;
        KEYCHAIN_CACHE
            .lock()
            .expect("keychain cache poisoned")
            .insert(key.to_owned(), Some(value.to_vec()));
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(APP_NAME, key)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(err) => return Err(err.into()),
        }
        KEYCHAIN_CACHE
            .lock()
            .expect("keychain cache poisoned")
            .insert(key.to_owned(), None);
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct MemorySecretStore {
    entries: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .entries
            .lock()
            .expect("memory secret store poisoned")
            .get(key)
            .cloned())
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        self.entries
            .lock()
            .expect("memory secret store poisoned")
            .insert(key.to_owned(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.entries
            .lock()
            .expect("memory secret store poisoned")
            .remove(key);
        Ok(())
    }
}

pub fn ensure_identity(store: &dyn SecretStore) -> Result<Identity> {
    let mut vault = load_vault(store)?;
    if let Some(identity) = vault.identity.clone() {
        return Ok(identity.into());
    }

    let identity = generate_identity()?;
    vault.identity = Some(identity.clone().into());
    store_vault(store, &vault)?;
    Ok(identity)
}

pub fn generate_identity() -> Result<Identity> {
    let mut random = [0_u8; 16];
    OsRng.fill_bytes(&mut random);
    let device_id = hex::encode(random);
    let signing_key = KeyPair::generate()?;
    let mut params = CertificateParams::new(vec!["rclippy.local".to_owned()])?;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    let cert = params.self_signed(&signing_key)?;

    Ok(Identity {
        device_id,
        cert_der: cert.der().as_ref().to_vec(),
        key_der: signing_key.serialize_der(),
    })
}

pub fn load_peer(store: &dyn SecretStore) -> Result<Option<PeerIdentity>> {
    Ok(load_vault(store)?.peer.map(Into::into))
}

pub fn store_peer(store: &dyn SecretStore, peer: &PeerIdentity) -> Result<()> {
    let mut vault = load_vault(store)?;
    vault.peer = Some(peer.clone().into());
    store_vault(store, &vault)?;
    Ok(())
}

pub fn delete_peer(store: &dyn SecretStore) -> Result<()> {
    let mut vault = load_vault(store)?;
    vault.peer = None;
    store_vault(store, &vault)?;
    Ok(())
}

fn load_vault(store: &dyn SecretStore) -> Result<StoredVault> {
    let Some(vault) = store.get(VAULT_KEY)? else {
        return Ok(StoredVault::default());
    };
    serde_json::from_slice(&vault).context("stored secret vault is invalid")
}

fn store_vault(store: &dyn SecretStore, vault: &StoredVault) -> Result<()> {
    store.set(VAULT_KEY, &serde_json::to_vec(vault)?)?;
    Ok(())
}

pub fn cert_fingerprint(cert_der: &[u8]) -> String {
    hex::encode(Sha256::digest(cert_der))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_persists_in_store() {
        let store = MemorySecretStore::default();

        let first = ensure_identity(&store).unwrap();
        let second = ensure_identity(&store).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.cert_fingerprint().len(), 64);
    }

    #[test]
    fn peer_round_trip() {
        let store = MemorySecretStore::default();
        let peer = PeerIdentity {
            device_id: "peer".to_owned(),
            device_name: "workstation".to_owned(),
            cert_der: vec![1, 2, 3],
        };

        store_peer(&store, &peer).unwrap();
        assert_eq!(load_peer(&store).unwrap(), Some(peer));

        delete_peer(&store).unwrap();
        assert_eq!(load_peer(&store).unwrap(), None);
    }
}
