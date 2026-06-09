//! `ghal_bol` is the **core** of Ghal Bol: keystore, P2P, messaging, contacts, transcripts, invites.
//!
//! **Flutter (`ghal_bol_ui`) is a thin UI** — see workspace `AGENTS.md` and `docs/ARCHITECTURE.md`.
//!
//! v1 design goal: one secp256k1 keypair (libp2p PeerId, sign, encrypt),
//! encrypted-at-rest with a key derived from an app-level password. The password is used
//! only to unlock local storage; it is never shared.
//!
//! On **native** targets (non-Wasm), [`p2p`] runs a **libp2p DM node** with **coord lookup**,
//! **mDNS/LAN**, and optional **bootstrap dial addrs** from connect invites
//! ([`p2p::GossipChatConfig::bootstrap_peers`]). The keystore compiles on Wasm without the P2P stack.
//!
//! **Android:** the Flutter app and [`ANDROID_LIBRARY_NAMESPACE`] both use **`com.ghalbol`**
//! for packaging and the keystore data-directory root.

mod app_paths;
mod flow_log;
mod preferences_v1;
#[cfg(not(target_arch = "wasm32"))]
mod preferences_ffi;
mod c_ffi;
mod connect_invite_v1;
mod contacts_v1;
mod contacts_ffi;
mod dm_event_handler;
mod dm_transcript_store;
mod invite_ffi;
mod keystore_v1;
mod dm_transcript_v1;
mod msg_v1;
#[cfg(not(target_arch = "wasm32"))]
mod call_sig_v1;
#[cfg(not(target_arch = "wasm32"))]
mod call_media_key;
#[cfg(not(target_arch = "wasm32"))]
mod call_ffi;
#[cfg(not(target_arch = "wasm32"))]
mod call_state;
#[cfg(not(target_arch = "wasm32"))]
mod call_media;
#[cfg(not(target_arch = "wasm32"))]
mod call_video;
mod peer_id_util;
mod public_key_util;
#[cfg(not(target_arch = "wasm32"))]
pub mod coord;
#[cfg(not(target_arch = "wasm32"))]
pub mod coord_runtime;
#[cfg(not(target_arch = "wasm32"))]
mod coord_ffi;
#[cfg(not(target_arch = "wasm32"))]
mod dm_transport;
#[cfg(not(target_arch = "wasm32"))]
mod p2p_runtime;
#[cfg(not(target_arch = "wasm32"))]
mod p2p_ffi;
#[cfg(not(target_arch = "wasm32"))]
mod session_runtime;
#[cfg(all(not(target_arch = "wasm32"), unix))]
pub mod daemon;
#[cfg(not(target_arch = "wasm32"))]
mod incoming_call_notify;
#[cfg(target_os = "android")]
mod android_daemon;
#[cfg(target_os = "android")]
mod android_jni_cache;
#[cfg(not(target_arch = "wasm32"))]
mod transcript_ffi;
mod storage;
mod secp256k1_seal;

pub use keystore_v1::{
    DecryptedIdentity, KeystoreError, KeystoreV1, KeystoreV1KdfParams, Libp2pIdentityError,
    create_keystore_v1, create_keystore_v1_from_secret, parse_secret_key_hex,
    secret_key_hex_from_identity, unlock_keystore_v1,
};

pub use storage::{
    ANDROID_LIBRARY_NAMESPACE, KeystoreStorageError, StorageConfig, StoredKeystore,
    create_or_unlock_identity_v1, delete_stored_identity_v1, export_keystore_json_v1,
    import_identity_from_secret_hex_v1, import_keystore_from_json_v1, keystore_v1_file_exists,
    load_keystore_v1, project_dirs_for_library, reset_first_time_identity_v1,
    reveal_secret_key_hex_v1, save_keystore_v1,
};

#[cfg(not(target_arch = "wasm32"))]
pub mod p2p;
#[cfg(not(target_arch = "wasm32"))]
pub use dm_transport::DmDialAddr;
