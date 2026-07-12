//! Store integration tests — opaque mailbox, sender isolation, TTL/quota policy.

use ghal_bol_delivery::{
    AppState, DeliveryConfig, DeliveryError, MailboxStore, PolicyLimits, ValidatedEnvelope,
};
use std::sync::Arc;

fn test_store() -> (Arc<MailboxStore>, PolicyLimits) {
    let cfg = DeliveryConfig::default();
    let state = AppState::new_in_memory(cfg).unwrap();
    (Arc::clone(&state.store), state.policy.clone())
}

fn env(sender: &str, recipient: &str, message_id: &str, blob: &str) -> ValidatedEnvelope {
    ValidatedEnvelope {
        message_id: message_id.into(),
        sender_wire: sender.into(),
        recipient_wire: recipient.into(),
        envelope_blob: blob.into(),
        size_bytes: blob.len() as i64,
    }
}

#[test]
fn opaque_storage_and_recipient_fetch() {
    let (store, policy) = test_store();
    let sender = "02".repeat(33);
    let recipient = "03".repeat(33);
    let blob = r#"{"ciphertext_hex":"deadbeef"}"#;
    store
        .upload(env(&sender, &recipient, "m1", blob), 3600, &policy)
        .unwrap();
    let pending = store.pending_for_recipient(&recipient).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, blob);
    assert_eq!(pending[0].1, "m1");
}

#[test]
fn sender_isolation_extend_wrong_owner() {
    let (store, policy) = test_store();
    let sender = "02".repeat(33);
    let other = "04".repeat(33);
    let recipient = "03".repeat(33);
    store
        .upload(env(&sender, &recipient, "m1", "{}"), 3600, &policy)
        .unwrap();
    let err = store
        .extend_ttl(&other, "m1", 7200, &policy)
        .unwrap_err();
    assert!(matches!(err, DeliveryError::NotFound(_)));
}

#[test]
fn resend_replace_keeps_single_row_and_quota() {
    let (store, policy) = test_store();
    let sender = "02".repeat(33);
    let recipient = "03".repeat(33);
    store
        .upload(env(&sender, &recipient, "m1", "aaaa"), 3600, &policy)
        .unwrap();
    let (_, replaced_first) = store
        .upload(env(&sender, &recipient, "m1", "bbbbbb"), 3600, &policy)
        .unwrap();
    assert!(replaced_first);
    let (quota_after_replace, replaced) = store
        .upload(env(&sender, &recipient, "m1", "cccc"), 3600, &policy)
        .unwrap();
    assert!(replaced);
    let rows = store.list_outbox(&sender, false).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].size_bytes, 4);
    assert_eq!(quota_after_replace.used_bytes, 4);
}

#[test]
fn extend_ttl_bounds_and_expired_resend() {
    let (store, policy) = test_store();
    let sender = "02".repeat(33);
    let recipient = "03".repeat(33);
    store
        .upload(env(&sender, &recipient, "m1", "{}"), 3600, &policy)
        .unwrap();
    let row = store.extend_ttl(&sender, "m1", 7200, &policy).unwrap();
    assert!(row.expires_at_ms > 0);
    let err = store.extend_ttl(&sender, "m1", 1, &policy).unwrap_err();
    assert!(matches!(err, DeliveryError::TtlInvalid(_)));

    store.test_force_expires_at(&sender, "m1", 1).unwrap();
    store.sweep_expired().unwrap();
    let expired_err = store.extend_ttl(&sender, "m1", 7200, &policy).unwrap_err();
    assert!(matches!(expired_err, DeliveryError::Expired(_)));

    store
        .upload(env(&sender, &recipient, "m1", "{\"resend\":true}"), 3600, &policy)
        .unwrap();
    let rows = store.list_outbox(&sender, true).unwrap();
    assert_eq!(rows[0].state, "queued");
}

#[test]
fn ack_deliver_frees_quota() {
    let (store, policy) = test_store();
    let sender = "02".repeat(33);
    let recipient = "03".repeat(33);
    store
        .upload(env(&sender, &recipient, "m1", "payload"), 3600, &policy)
        .unwrap();
    let before = store.quota_status(&sender).unwrap();
    assert_eq!(before.pending_count, 1);
    store.ack_deliver(&recipient, "m1", &sender).unwrap();
    let after = store.quota_status(&sender).unwrap();
    assert_eq!(after.pending_count, 0);
    assert_eq!(after.used_bytes, 0);
}
