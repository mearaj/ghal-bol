//! Per-algorithm identity signatures (envelope auth only — not transport ciphertext).

use ed25519_dalek::{Signature as Ed25519Sig, Signer, Verifier, VerifyingKey};
use p256::ecdsa::{
    Signature as EcdsaSig, VerifyingKey as EcdsaVerifyingKey,
    signature::Verifier as EcdsaVerifier,
};
use secp256k1::{ecdsa::Signature as Secp256k1Sig, Message, Secp256k1};
use sha2::{Digest, Sha256};

use crate::identity::{Identity, IdentityAlgorithm};
use crate::keystore_v1::DecryptedIdentity;

fn secp256k1_sign(secret: &secp256k1::SecretKey, msg: &[u8]) -> Result<Vec<u8>, String> {
    let secp = Secp256k1::new();
    let digest: [u8; 32] = Sha256::digest(msg).into();
    let message = Message::from_digest(digest);
    Ok(secp
        .sign_ecdsa(message, secret)
        .serialize_compact()
        .to_vec())
}

fn secp256k1_verify(pk_bytes: &[u8], msg: &[u8], signature: &[u8]) -> Result<(), String> {
    let secp = Secp256k1::new();
    let pk = secp256k1::PublicKey::from_slice(pk_bytes)
        .map_err(|e| format!("secp256k1 public key: {e}"))?;
    let digest: [u8; 32] = Sha256::digest(msg).into();
    let message = Message::from_digest(digest);
    let sig = Secp256k1Sig::from_compact(signature)
        .map_err(|e| format!("secp256k1 signature: {e}"))?;
    secp.verify_ecdsa(message, &sig, &pk)
        .map_err(|e| format!("secp256k1 verify: {e}"))?;
    Ok(())
}

impl DecryptedIdentity {
    /// Sign a message with this identity's private key (algorithm-specific wire format).
    pub fn sign_message(&self, msg: &[u8]) -> Result<Vec<u8>, String> {
        match self.algorithm() {
            IdentityAlgorithm::Secp256k1 => secp256k1_sign(self.secp256k1_secret(), msg),
            IdentityAlgorithm::Ed25519 => {
                let signing = self
                    .ed25519_signing_key()
                    .ok_or_else(|| "ed25519 signing key unavailable".to_string())?;
                Ok(signing.sign(msg).to_bytes().to_vec())
            }
            IdentityAlgorithm::EcdsaP256 => {
                use p256::ecdsa::signature::Signer as EcdsaSigner;
                let signing = self
                    .ecdsa_p256_signing_key()
                    .ok_or_else(|| "ecdsa-p256 signing key unavailable".to_string())?;
                let sig: EcdsaSig = EcdsaSigner::sign(signing, msg);
                Ok(sig.to_bytes().to_vec())
            }
        }
    }
}

/// Verify envelope signature bytes against sender identity wire.
pub fn verify_identity_signature(
    sender_identity_wire: &str,
    msg: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    let id = Identity::parse(sender_identity_wire)?;
    match id.algorithm {
        IdentityAlgorithm::Secp256k1 => secp256k1_verify(&id.public_key, msg, signature),
        IdentityAlgorithm::Ed25519 => {
            let arr: [u8; 32] = id
                .public_key
                .as_slice()
                .try_into()
                .map_err(|_| "ed25519 public key: invalid length".to_string())?;
            let vk = VerifyingKey::from_bytes(&arr)
                .map_err(|e| format!("ed25519 public key: {e}"))?;
            let sig = Ed25519Sig::from_slice(signature)
                .map_err(|e| format!("ed25519 signature: {e}"))?;
            vk.verify(msg, &sig)
                .map_err(|e| format!("ed25519 verify: {e}"))?;
            Ok(())
        }
        IdentityAlgorithm::EcdsaP256 => {
            let vk = EcdsaVerifyingKey::from_sec1_bytes(&id.public_key)
                .map_err(|e| format!("ecdsa-p256 public key: {e}"))?;
            let sig = EcdsaSig::from_der(signature)
                .or_else(|_| EcdsaSig::from_slice(signature))
                .map_err(|e| format!("ecdsa-p256 signature: {e}"))?;
            EcdsaVerifier::verify(&vk, msg, &sig)
                .map_err(|e| format!("ecdsa-p256 verify: {e}"))?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1_with_algorithm;

    #[test]
    fn secp256k1_sign_verify_roundtrip() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Secp256k1, None).unwrap();
        let msg = b"ghal_bol envelope canonical";
        let sig = id.sign_message(msg).unwrap();
        verify_identity_signature(&id.identity_wire(), msg, &sig).unwrap();
    }

    #[test]
    fn ed25519_sign_verify_roundtrip() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Ed25519, None).unwrap();
        let msg = b"ghal_bol envelope canonical";
        let sig = id.sign_message(msg).unwrap();
        verify_identity_signature(&id.identity_wire(), msg, &sig).unwrap();
    }

    #[test]
    fn ecdsa_p256_sign_verify_roundtrip() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::EcdsaP256, None).unwrap();
        let msg = b"ghal_bol envelope canonical";
        let sig = id.sign_message(msg).unwrap();
        verify_identity_signature(&id.identity_wire(), msg, &sig).unwrap();
    }
}
