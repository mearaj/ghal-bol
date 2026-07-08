//! Per-algorithm identity signatures (envelope auth only — not transport ciphertext).

use ed25519_dalek::{Signature as Ed25519Sig, Signer, Verifier, VerifyingKey};
use p256::ecdsa::{
    Signature as EcdsaSig, VerifyingKey as EcdsaVerifyingKey,
    signature::Verifier as EcdsaVerifier,
};

use crate::identity::{Identity, IdentityAlgorithm};
use crate::keystore_v1::DecryptedIdentity;

impl DecryptedIdentity {
    /// Sign a message with this identity's private key (algorithm-specific wire format).
    pub fn sign_message(&self, msg: &[u8]) -> Result<Vec<u8>, String> {
        match self.algorithm() {
            IdentityAlgorithm::Secp256k1 => {
                let sig = self
                    .keypair()
                    .sign(msg)
                    .map_err(|e| format!("secp256k1 sign: {e}"))?;
                Ok(sig)
            }
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
            IdentityAlgorithm::MlDsa65 => {
                let signing = self
                    .ml_dsa65_signing_key()
                    .ok_or_else(|| "ml-dsa-65 signing key unavailable".to_string())?;
                crate::ml_dsa_identity::sign_message(signing, msg)
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
        IdentityAlgorithm::Secp256k1 => {
            let pk = libp2p_identity::secp256k1::PublicKey::try_from_bytes(&id.public_key)
                .map_err(|e| format!("secp256k1 public key: {e}"))?;
            let libp2p_pk = libp2p_identity::PublicKey::from(pk);
            if !libp2p_pk.verify(msg, signature) {
                return Err("signature verification failed".to_string());
            }
            Ok(())
        }
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
        IdentityAlgorithm::MlDsa65 => {
            crate::ml_dsa_identity::verify_message(&id.public_key, msg, signature)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1_with_algorithm;

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

    #[test]
    fn ml_dsa65_sign_verify_roundtrip() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::MlDsa65, None).unwrap();
        let msg = b"ghal_bol envelope canonical";
        let sig = id.sign_message(msg).unwrap();
        verify_identity_signature(&id.identity_wire(), msg, &sig).unwrap();
    }
}
