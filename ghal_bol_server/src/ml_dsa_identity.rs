//! ML-DSA-65 helpers for coord registration and identity validation.

use ml_dsa::{
    EncodedSignature, EncodedVerifyingKey, Generate, Keypair, MlDsa65, Seed,
    Signature, SignatureEncoding, SigningKey, Signer, Verifier, VerifyingKey,
};

pub const SEED_LEN: usize = 32;
pub const PUBLIC_KEY_LEN: usize = 1952;
pub const SIGNATURE_LEN: usize = 3309;

pub fn generate_secret_seed() -> [u8; SEED_LEN] {
    SigningKey::<MlDsa65>::generate().to_seed().into()
}

pub fn signing_key_from_seed_bytes(seed: &[u8]) -> Result<SigningKey<MlDsa65>, String> {
    if seed.len() != SEED_LEN {
        return Err(format!("ml-dsa-65 seed must be {SEED_LEN} bytes"));
    }
    let mut arr = Seed::default();
    arr.copy_from_slice(seed);
    Ok(SigningKey::from_seed(&arr))
}

pub fn public_key_bytes_from_seed(seed: &[u8]) -> Result<Vec<u8>, String> {
    let sk = signing_key_from_seed_bytes(seed)?;
    Ok(sk.verifying_key().encode().as_slice().to_vec())
}

pub fn validate_public_key_bytes(pk: &[u8]) -> Result<(), String> {
    if pk.len() != PUBLIC_KEY_LEN {
        return Err(format!("ml-dsa-65 public key: expected {PUBLIC_KEY_LEN} bytes"));
    }
    let enc = EncodedVerifyingKey::<MlDsa65>::try_from(pk)
        .map_err(|_| "ml-dsa-65 public key: invalid encoding".to_string())?;
    VerifyingKey::<MlDsa65>::decode(&enc);
    Ok(())
}

pub fn sign_message(signing: &SigningKey<MlDsa65>, msg: &[u8]) -> Result<Vec<u8>, String> {
    let sig = signing.sign(msg);
    Ok(sig.to_bytes().as_slice().to_vec())
}

pub fn verify_message(pk: &[u8], msg: &[u8], signature: &[u8]) -> Result<(), String> {
    if signature.len() != SIGNATURE_LEN {
        return Err(format!("ml-dsa-65 signature: expected {SIGNATURE_LEN} bytes"));
    }
    let enc_vk = EncodedVerifyingKey::<MlDsa65>::try_from(pk)
        .map_err(|_| "ml-dsa-65 public key: invalid encoding".to_string())?;
    let vk = VerifyingKey::<MlDsa65>::decode(&enc_vk);
    let enc_sig = EncodedSignature::<MlDsa65>::try_from(signature)
        .map_err(|_| "ml-dsa-65 signature: invalid encoding".to_string())?;
    let sig = Signature::<MlDsa65>::decode(&enc_sig)
        .ok_or_else(|| "ml-dsa-65 signature: decode failed".to_string())?;
    vk.verify(msg, &sig)
        .map_err(|e| format!("ml-dsa-65 verify: {e}"))
}
