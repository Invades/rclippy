use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use rand::{RngCore, rngs::OsRng};
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};

use crate::APP_NAME;

const DEVICE_ID_KEY: &str = "device-id";
const CERT_DER_KEY: &str = "local-cert-der";
const KEY_DER_KEY: &str = "local-key-pkcs8-der";
const PEER_DEVICE_ID_KEY: &str = "peer-device-id";
const PEER_CERT_DER_KEY: &str = "peer-cert-der";

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
    pub cert_der: Vec<u8>,
}

impl PeerIdentity {
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
        let entry = keyring::Entry::new(APP_NAME, key)?;
        match entry.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        let entry = keyring::Entry::new(APP_NAME, key)?;
        entry.set_secret(value)?;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(APP_NAME, key)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
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
    let existing = match (
        store.get(DEVICE_ID_KEY)?,
        store.get(CERT_DER_KEY)?,
        store.get(KEY_DER_KEY)?,
    ) {
        (Some(device_id), Some(cert_der), Some(key_der)) => Some(Identity {
            device_id: String::from_utf8(device_id).context("stored device id is not UTF-8")?,
            cert_der,
            key_der,
        }),
        _ => None,
    };

    if let Some(identity) = existing {
        return Ok(identity);
    }

    let identity = generate_identity()?;
    store.set(DEVICE_ID_KEY, identity.device_id.as_bytes())?;
    store.set(CERT_DER_KEY, &identity.cert_der)?;
    store.set(KEY_DER_KEY, &identity.key_der)?;
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
    let Some(device_id) = store.get(PEER_DEVICE_ID_KEY)? else {
        return Ok(None);
    };
    let Some(cert_der) = store.get(PEER_CERT_DER_KEY)? else {
        return Ok(None);
    };

    Ok(Some(PeerIdentity {
        device_id: String::from_utf8(device_id).context("stored peer device id is not UTF-8")?,
        cert_der,
    }))
}

pub fn store_peer(store: &dyn SecretStore, peer: &PeerIdentity) -> Result<()> {
    store.set(PEER_DEVICE_ID_KEY, peer.device_id.as_bytes())?;
    store.set(PEER_CERT_DER_KEY, &peer.cert_der)?;
    Ok(())
}

pub fn delete_peer(store: &dyn SecretStore) -> Result<()> {
    store.delete(PEER_DEVICE_ID_KEY)?;
    store.delete(PEER_CERT_DER_KEY)?;
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
            cert_der: vec![1, 2, 3],
        };

        store_peer(&store, &peer).unwrap();
        assert_eq!(load_peer(&store).unwrap(), Some(peer));

        delete_peer(&store).unwrap();
        assert_eq!(load_peer(&store).unwrap(), None);
    }
}
