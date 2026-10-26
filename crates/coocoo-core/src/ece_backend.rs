//! Pure-Rust crypto backend for `ece` (RFC 8188), used by the push sender.
//!
//! The `web-push` crate depends on `ece` whose default backend is OpenSSL. Headless servers
//! (and this build) must not require libssl; instead `ece` accepts a global [`Cryptographer`].
//! This one is p256 + RustCrypto (`aes-gcm`, `hkdf`, `sha2`) and covers exactly what
//! `web_push::WebPushMessageBuilder` needs to encrypt an `aes128gcm` payload.

use aes_gcm::aead::consts::U12;
use aes_gcm::aead::Aead;
use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
use ece::crypto::{Cryptographer, EcKeyComponents, LocalKeyPair, RemotePublicKey};
use ece::{Error, Result};
use hkdf::Hkdf;
use p256::elliptic_curve::ecdh::diffie_hellman;
use p256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use p256::{EncodedPoint, PublicKey, SecretKey};
use sha2::Sha256;
use std::any::Any;

struct RemoteKey(PublicKey);
struct LocalKey(SecretKey);

impl RemotePublicKey for RemoteKey {
    fn as_raw(&self) -> Result<Vec<u8>> {
        Ok(self.0.to_encoded_point(false).as_bytes().to_vec())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LocalKeyPair for LocalKey {
    fn pub_as_raw(&self) -> Result<Vec<u8>> {
        Ok(self.0.public_key().to_encoded_point(false).as_bytes().to_vec())
    }
    fn raw_components(&self) -> Result<EcKeyComponents> {
        let public = self.pub_as_raw()?;
        Ok(EcKeyComponents::new(self.0.to_bytes().as_slice(), public.as_slice()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct RustCryptographer;

impl Cryptographer for RustCryptographer {
    fn generate_ephemeral_keypair(&self) -> Result<Box<dyn LocalKeyPair>> {
        Ok(Box::new(LocalKey(SecretKey::random(&mut rand_core::OsRng))))
    }

    fn import_key_pair(&self, components: &EcKeyComponents) -> Result<Box<dyn LocalKeyPair>> {
        let secret = SecretKey::from_slice(components.private_key()).map_err(|_| Error::CryptoError)?;
        Ok(Box::new(LocalKey(secret)))
    }

    fn import_public_key(&self, raw: &[u8]) -> Result<Box<dyn RemotePublicKey>> {
        let point = EncodedPoint::from_bytes(raw).map_err(|_| Error::CryptoError)?;
        let key = PublicKey::from_encoded_point(&point).into_option().ok_or(Error::CryptoError)?;
        Ok(Box::new(RemoteKey(key)))
    }

    fn compute_ecdh_secret(&self, remote: &dyn RemotePublicKey, local: &dyn LocalKeyPair) -> Result<Vec<u8>> {
        let remote = remote.as_any().downcast_ref::<RemoteKey>().ok_or(Error::CryptoError)?;
        let local = local.as_any().downcast_ref::<LocalKey>().ok_or(Error::CryptoError)?;
        Ok(diffie_hellman(local.0.to_nonzero_scalar(), remote.0.as_affine()).raw_secret_bytes().to_vec())
    }

    fn hkdf_sha256(&self, salt: &[u8], secret: &[u8], info: &[u8], len: usize) -> Result<Vec<u8>> {
        let (_, hk) = Hkdf::<Sha256>::extract(Some(salt), secret);
        let mut okm = vec![0u8; len];
        hk.expand(info, &mut okm).map_err(|_| Error::CryptoError)?;
        Ok(okm)
    }

    fn aes_gcm_128_encrypt(&self, key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes128Gcm::new_from_slice(key).map_err(|_| Error::CryptoError)?;
        cipher.encrypt(Nonce::<U12>::from_slice(iv), data).map_err(|_| Error::CryptoError)
    }

    fn aes_gcm_128_decrypt(&self, key: &[u8], iv: &[u8], ciphertext_and_tag: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes128Gcm::new_from_slice(key).map_err(|_| Error::CryptoError)?;
        cipher.decrypt(Nonce::<U12>::from_slice(iv), ciphertext_and_tag).map_err(|_| Error::CryptoError)
    }

    fn random_bytes(&self, dest: &mut [u8]) -> Result<()> {
        use rand_core::RngCore;
        rand_core::OsRng.fill_bytes(dest);
        Ok(())
    }
}

/// Installs the backend once. Panics of a second call are swallowed (idempotent).
pub(crate) fn install() {
    let _ = ece::crypto::set_boxed_cryptographer(Box::new(RustCryptographer));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ece's own backend suite (RFC 8188 vectors) against our Rust implementation.
    #[test]
    fn rfc_vectors() {
        install();
        ece::crypto::test_cryptographer(RustCryptographer);
    }

    /// What the push sender does: encrypt with a subscription's keys, the recipient side
    /// of ece decrypts it back (both through the installed global backend).
    #[test]
    fn roundtrip_through_global_backend() {
        install();
        let (local, auth) = ece::generate_keypair_and_auth_secret().unwrap();
        let remote_public = local.pub_as_raw().unwrap();
        let components = local.raw_components().unwrap();
        let plaintext = b"Agent needs you";
        let ciphertext = ece::encrypt(&remote_public, &auth, plaintext).unwrap();
        let back = ece::decrypt(&components, &auth, &ciphertext).unwrap();
        assert_eq!(back, plaintext);
    }
}
