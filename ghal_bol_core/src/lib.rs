//! `ghal_bol` is the **core** of Ghal Bol: keystore, P2P, messaging, contacts, transcripts, invites.
//!
//! **Flutter (`ghal_bol_ui`) is a thin UI** — see workspace `AGENTS.md` and `docs/ARCHITECTURE.md`.
//!
//! v1 design goal: one secp256k1 keypair (libp2p PeerId, sign, encrypt),
//! encrypted-at-rest with a key derived from an app-level password. The password is used
//! only to unlock local storage; it is never shared.
//!
//! On **native** targets (non-Wasm), [`p2p`] runs a **libp2p node** for **LAN text**, **voice/video calls**
//! (WAN + LAN), with **coord + relay** for WAN call reachability and **mDNS** for LAN.
//! **WAN text** uses [`delivery_runtime`] + [`text_transport`] (E2E encrypted delivery server).
//!
//! **Android:** the Flutter app and [`ANDROID_LIBRARY_NAMESPACE`] both use **`com.ghalbol`**
//! for packaging and the keystore data-directory root.

#[cfg(target_os = "android")]
mod android_daemon;
#[cfg(target_os = "android")]
mod android_jni_cache;
#[cfg(target_os = "android")]
mod android_network;
mod app_paths;
#[cfg(not(target_arch = "wasm32"))]
pub use app_paths::detect_keystore_app_namespace;
mod c_ffi;
#[cfg(not(target_arch = "wasm32"))]
mod call_ffi;
#[cfg(not(target_arch = "wasm32"))]
mod call_media;
#[cfg(not(target_arch = "wasm32"))]
mod call_media_key;
#[cfg(not(target_arch = "wasm32"))]
mod offline_seal_v1;
#[cfg(not(target_arch = "wasm32"))]
mod call_sig_v1;
#[cfg(not(target_arch = "wasm32"))]
mod call_state;
#[cfg(not(target_arch = "wasm32"))]
mod call_video;
mod connect_invite_v1;
mod contacts_ffi;
mod contacts_v1;
#[cfg(not(target_arch = "wasm32"))]
pub mod coord;
#[cfg(not(target_arch = "wasm32"))]
mod connect;
#[cfg(not(target_arch = "wasm32"))]
mod coord_ffi;
#[cfg(not(target_arch = "wasm32"))]
mod coord_register_auth;
#[cfg(not(target_arch = "wasm32"))]
pub mod delivery_auth;
#[cfg(not(target_arch = "wasm32"))]
mod delivery_client;
#[cfg(not(target_arch = "wasm32"))]
pub mod delivery_msg_v1;
#[cfg(not(target_arch = "wasm32"))]
mod delivery_read_acks;
#[cfg(not(target_arch = "wasm32"))]
mod delivery_runtime;
#[cfg(not(target_arch = "wasm32"))]
mod text_transport;
#[cfg(not(target_arch = "wasm32"))]
pub mod coord_runtime;
#[cfg(not(target_arch = "wasm32"))]
mod wan_coord;
#[cfg(all(not(target_arch = "wasm32"), unix))]
pub mod daemon;
#[cfg(not(target_arch = "wasm32"))]
mod transcript_ffi;
mod dm_event_handler;
mod session_key_common;
mod dm_transcript_store;
mod dm_transcript_v1;
#[cfg(not(target_arch = "wasm32"))]
mod dm_transport;
mod flow_log;
#[cfg(target_os = "android")]
mod incoming_call_android;
#[cfg(not(target_arch = "wasm32"))]
mod incoming_call_notify;
mod identity;
mod identity_ffi;
mod identity_sign;
mod invite_ffi;
mod keystore_v1;
#[cfg(target_os = "linux")]
mod linux_desktop_launch;
#[cfg(target_os = "linux")]
pub use linux_desktop_launch::{wake_for_incoming_call, wake_for_unlock};
#[cfg(target_os = "linux")]
mod linux_network;
mod msg_v1;
mod multiaddr_local;
#[cfg(not(target_arch = "wasm32"))]
mod network_ffi;
#[cfg(not(target_arch = "wasm32"))]
mod p2p_ffi;
#[cfg(not(target_arch = "wasm32"))]
mod p2p_runtime;
mod peer_id_util;
#[cfg(not(target_arch = "wasm32"))]
mod preferences_ffi;
mod preferences_v1;
mod public_key_util;
#[cfg(not(target_arch = "wasm32"))]
pub mod rustls_init;
mod symmetric_seal;
#[cfg(not(target_arch = "wasm32"))]
mod session_runtime;
mod storage;
#[cfg(not(target_arch = "wasm32"))]
mod transport_kem_v1;

pub use identity::{Identity, IdentityAlgorithm, normalize_identity_wire, same_contact_identity};
pub use keystore_v1::{
    DecryptedIdentity, KeystoreError, KeystoreV1, KeystoreV1KdfParams, Libp2pIdentityError,
    create_keystore_v1, create_keystore_v1_from_secret,
    create_keystore_v1_from_secret_with_algorithm, create_keystore_v1_with_algorithm,
    parse_secret_key_hex, secret_key_hex_from_identity, unlock_keystore_v1,
};

pub use storage::{
    ANDROID_LIBRARY_NAMESPACE, KeystoreStorageError, StorageConfig, StoredKeystore,
    create_or_unlock_identity_v1, create_or_unlock_identity_v1_with_algorithm,
    delete_stored_identity_v1, export_keystore_json_v1,
    import_identity_from_secret_hex_v1, import_identity_from_secret_hex_v1_with_algorithm,
    import_keystore_from_json_v1, keystore_v1_file_exists,
    load_keystore_v1, project_dirs_for_library, reset_first_time_identity_v1,
    reveal_secret_key_hex_v1, save_keystore_v1,
};

#[cfg(not(target_arch = "wasm32"))]
pub mod p2p;
#[cfg(not(target_arch = "wasm32"))]
pub use dm_event_handler::set_p2p_handler_context;
#[cfg(not(target_arch = "wasm32"))]
pub use session_runtime::session_unlocked;
#[cfg(not(target_arch = "wasm32"))]
pub use dm_transport::DmDialAddr;
