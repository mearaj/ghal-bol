//! **`ghal_bol_connect_v1`** connect invite — formats **2** and **3**.
//!
//! **Invite URLs (current):**
//! - `https://ghalbol.com/connect/<identity_wire>`
//! - `ghalbol://connect/<identity_wire>`
//!
//! Optional query: `?alias=<display name>` (URL-encoded).

use serde_json::Value;

use crate::identity::{percent_decode_uri_component, percent_encode_uri_component, Identity};
use crate::public_key_util::normalize_contact_identity_wire;

const SHARE: &str = "ghal_bol_connect_v1";
/// Legacy wire format (still accepted).
pub const CONNECT_INVITE_FORMAT_VERSION: u64 = 2;
/// Current emit version: `identity_wire` + `global_alias`.
pub const CONNECT_INVITE_FORMAT_VERSION_V3: u64 = 3;

/// HTTPS invite host (owned domain).
pub const CONNECT_HTTPS_HOST: &str = "ghalbol.com";
/// App deep-link scheme.
pub const CONNECT_APP_SCHEME: &str = "ghalbol";
/// Path segment after host or scheme: `/connect/<public_key_hex>`.
pub const CONNECT_PATH_SEGMENT: &str = "connect";

const DEFAULT_TOPIC: &str = "ghal-bol-chat";

fn identity_wire_from_field(label: &str, wire_s: &str) -> Result<String, String> {
    Identity::parse(wire_s).map_err(|e| format!("{label}: {e}"))?;
    normalize_contact_identity_wire(wire_s).map_err(|e| format!("{label}: {e}"))
}

fn field_nonempty_string(v: &Value, key: &str) -> bool {
    v.get(key)
        .and_then(|x| x.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

fn wire_share_from_value(v: &Value) -> Result<&str, String> {
    v.get("ghalbol.share")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing ghalbol.share".to_string())
}

fn verify_v3(v: &Value) -> Result<(), String> {
    let share = wire_share_from_value(v)?;
    if share != SHARE {
        return Err(format!("unknown ghalbol.share: {share}"));
    }
    let fv = v
        .get("format_version")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "missing format_version".to_string())?;
    if fv != CONNECT_INVITE_FORMAT_VERSION_V3 {
        return Err(format!(
            "unsupported format_version (expected {})",
            CONNECT_INVITE_FORMAT_VERSION_V3
        ));
    }
    if v.get("topic").and_then(|x| x.as_str()).is_none() {
        return Err("missing topic".to_string());
    }
    if field_nonempty_string(v, "invite_signature_hex") {
        return Err("invite_signature_hex is not supported".to_string());
    }
    if field_nonempty_string(v, "peer_id") {
        return Err("peer_id must not appear on wire".to_string());
    }
    if v.get("multiaddrs").is_some() {
        return Err("multiaddrs must not appear on wire".to_string());
    }
    let wire = v
        .get("identity_wire")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing identity_wire".to_string())?;
    identity_wire_from_field("identity_wire", wire)?;
    if v.get("public_key_hex").is_some() {
        return Err("public_key_hex must not appear on v3 wire (use identity_wire)".to_string());
    }
    if v.get("peer_alias").is_some() {
        return Err("peer_alias must not appear on v3 wire (use global_alias)".to_string());
    }
    if let Some(alias) = v.get("global_alias").and_then(|x| x.as_str()) {
        if alias.trim().is_empty() {
            return Err("global_alias must not be empty when present".to_string());
        }
    }
    Ok(())
}

fn verify_v2(v: &Value) -> Result<(), String> {
    let share = wire_share_from_value(v)?;
    if share != SHARE {
        return Err(format!("unknown ghalbol.share: {share}"));
    }
    let fv = v
        .get("format_version")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "missing format_version".to_string())?;
    if fv != CONNECT_INVITE_FORMAT_VERSION {
        return Err(format!(
            "unsupported format_version (expected {})",
            CONNECT_INVITE_FORMAT_VERSION
        ));
    }
    if v.get("topic").and_then(|x| x.as_str()).is_none() {
        return Err("missing topic".to_string());
    }
    if field_nonempty_string(v, "invite_signature_hex") {
        return Err("invite_signature_hex is not supported".to_string());
    }
    if field_nonempty_string(v, "peer_id") {
        return Err("peer_id must not appear on wire (use public_key_hex only)".to_string());
    }
    if v.get("multiaddrs").is_some() {
        return Err("multiaddrs must not appear on wire (use coordination lookup)".to_string());
    }
    let pk_hex = v
        .get("public_key_hex")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing public_key_hex".to_string())?;
    identity_wire_from_field("public_key_hex", pk_hex)?;
    if field_nonempty_string(v, "encryption_public_key_hex") {
        return Err("encryption_public_key_hex is not used (single secp256k1 key)".to_string());
    }
    if field_nonempty_string(v, "coord_base_url") {
        return Err(
            "coord_base_url must not appear on invite wire (use app env / preferences)".to_string(),
        );
    }
    Ok(())
}

/// Build format-3 wire map (`identity_wire` + optional `global_alias`).
pub fn build_connect_invite_wire_map(
    topic: &str,
    public_key_hex: &str,
    peer_alias: Option<&str>,
) -> Result<Value, String> {
    let wire = identity_wire_from_field("identity_wire", public_key_hex)?;
    let mut m = serde_json::json!({
        "ghalbol.share": SHARE,
        "format_version": CONNECT_INVITE_FORMAT_VERSION_V3,
        "topic": topic,
        "identity_wire": wire,
    });
    if let Some(a) = peer_alias {
        let t = a.trim();
        if !t.is_empty() {
            m["global_alias"] = Value::String(t.to_string());
        }
    }
    Ok(m)
}

/// Parse v2 or v3 wire and return normalized identity wire.
pub fn identity_wire_from_invite(v: &Value) -> Result<String, String> {
    verify_ghal_bol_connect_invite_value(v)?;
    let fv = v.get("format_version").and_then(|x| x.as_u64()).unwrap_or(0);
    if fv == CONNECT_INVITE_FORMAT_VERSION_V3 {
        let w = v
            .get("identity_wire")
            .and_then(|x| x.as_str())
            .ok_or_else(|| "missing identity_wire".to_string())?;
        identity_wire_from_field("identity_wire", w)
    } else {
        let pk = v
            .get("public_key_hex")
            .and_then(|x| x.as_str())
            .ok_or_else(|| "missing public_key_hex".to_string())?;
        identity_wire_from_field("public_key_hex", pk)
    }
}

/// Global alias from invite wire (v3 `global_alias`, v2 `peer_alias`).
pub fn global_alias_from_invite(v: &Value) -> Option<String> {
    if let Some(a) = v.get("global_alias").and_then(|x| x.as_str()) {
        let t = a.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    v.get("peer_alias")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn percent_encode_query_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn wire_map_from_public_key_and_alias(
    public_key_hex: &str,
    peer_alias: Option<&str>,
) -> Result<Value, String> {
    build_connect_invite_wire_map(DEFAULT_TOPIC, public_key_hex, peer_alias)
}

/// `https://ghalbol.com/connect/<identity_wire>`
pub fn connect_invite_https_uri_from_wire_map(v: &Value) -> Result<String, String> {
    verify_ghal_bol_connect_invite_value(v)?;
    let pk = identity_wire_from_invite(v)?;
    let path_pk = percent_encode_uri_component(&pk);
    let mut url = format!("https://{CONNECT_HTTPS_HOST}/{CONNECT_PATH_SEGMENT}/{path_pk}");
    if let Some(alias) = global_alias_from_invite(v) {
        url.push('?');
        url.push_str(&format!("alias={}", percent_encode_query_component(&alias)));
    }
    Ok(url)
}

/// `ghalbol://connect/<identity_wire>`
pub fn connect_invite_app_uri_from_wire_map(v: &Value) -> Result<String, String> {
    verify_ghal_bol_connect_invite_value(v)?;
    let pk = identity_wire_from_invite(v)?;
    let path_pk = percent_encode_uri_component(&pk);
    let mut url = format!("{CONNECT_APP_SCHEME}://{CONNECT_PATH_SEGMENT}/{path_pk}");
    if let Some(alias) = global_alias_from_invite(v) {
        url.push('?');
        url.push_str(&format!("alias={}", percent_encode_query_component(&alias)));
    }
    Ok(url)
}

/// Primary invite URI for QR / share (HTTPS).
pub fn connect_invite_uri_from_wire_map(v: &Value) -> Result<String, String> {
    connect_invite_https_uri_from_wire_map(v)
}

fn parse_alias_query(query: Option<&str>) -> Option<String> {
    let q = query?;
    for part in q.split('&') {
        if let Some(v) = part.strip_prefix("alias=") {
            let decoded = percent_decode_query_component(v);
            let t = decoded.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn percent_decode_query_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_public_key_path_segment(seg: &str) -> Result<String, String> {
    let seg = percent_decode_uri_component(seg.trim().trim_end_matches('/'));
    identity_wire_from_field("public_key_hex", &seg)
}

fn parse_path_style_invite(input: &str) -> Result<Value, String> {
    let t = input.trim();
    let lower = t.to_ascii_lowercase();

    let (path_and_host, query) = match t.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (t, None),
    };
    let alias = parse_alias_query(query);

    // ghalbol://connect/<pk>
    let app_prefix = format!("{CONNECT_APP_SCHEME}://");
    if lower.starts_with(&app_prefix) {
        let rest = &path_and_host[app_prefix.len()..];
        let pk_seg = rest
            .strip_prefix(CONNECT_PATH_SEGMENT)
            .or_else(|| rest.strip_prefix(&format!("{CONNECT_PATH_SEGMENT}/")))
            .ok_or_else(|| "not a Ghal Bol connect invite".to_string())?
            .trim_start_matches('/');
        let pk = parse_public_key_path_segment(pk_seg)?;
        return wire_map_from_public_key_and_alias(&pk, alias.as_deref());
    }

    // https://ghalbol.com/connect/<pk>
    for scheme in ["https://", "http://"] {
        let prefix = format!("{scheme}{CONNECT_HTTPS_HOST}/");
        if lower.starts_with(&prefix.to_ascii_lowercase()) {
            let rest = &path_and_host[prefix.len()..];
            let pk_seg = rest
                .strip_prefix(CONNECT_PATH_SEGMENT)
                .or_else(|| rest.strip_prefix(&format!("{CONNECT_PATH_SEGMENT}/")))
                .ok_or_else(|| "not a Ghal Bol connect invite".to_string())?
                .trim_start_matches('/');
            let pk = parse_public_key_path_segment(pk_seg)?;
            return wire_map_from_public_key_and_alias(&pk, alias.as_deref());
        }
    }

    Err("not a Ghal Bol connect invite".to_string())
}

/// Parse invite URI → wire map (before verification).
pub fn parse_connect_invite_uri(input: &str) -> Result<Value, String> {
    let t = input.trim().replace([' ', '\n', '\r', '\t'], "");
    parse_path_style_invite(&t)
}

/// Verify **`ghal_bol_connect_v1`** JSON (formats **2** and **3**).
pub fn verify_ghal_bol_connect_invite_value(v: &Value) -> Result<(), String> {
    let fv = v
        .get("format_version")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "missing format_version".to_string())?;
    if fv == CONNECT_INVITE_FORMAT_VERSION_V3 {
        verify_v3(v)
    } else if fv == CONNECT_INVITE_FORMAT_VERSION {
        verify_v2(v)
    } else {
        Err(format!(
            "unsupported format_version (expected {} or {})",
            CONNECT_INVITE_FORMAT_VERSION, CONNECT_INVITE_FORMAT_VERSION_V3
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1;
    use crate::create_keystore_v1_with_algorithm;

    #[test]
    fn v2_ok() {
        let (_ks, id) = create_keystore_v1("pw", None).unwrap();
        let pk = id.public_key_hex();
        let v = serde_json::json!({
            "ghalbol.share": SHARE,
            "format_version": CONNECT_INVITE_FORMAT_VERSION,
            "topic": "ghal-bol-chat",
            "public_key_hex": pk,
        });
        verify_ghal_bol_connect_invite_value(&v).unwrap();
    }

    #[test]
    fn v2_rejects_multiaddrs_on_wire() {
        let (_ks, id) = create_keystore_v1("pw", None).unwrap();
        let pk = id.public_key_hex();
        let pid = id.identity_wire();
        let v = serde_json::json!({
            "ghalbol.share": SHARE,
            "format_version": CONNECT_INVITE_FORMAT_VERSION,
            "topic": "ghal-bol-chat",
            "public_key_hex": pk,
            "multiaddrs": [format!("/ip4/10.0.0.1/tcp/1234/p2p/{pid}")],
        });
        assert!(verify_ghal_bol_connect_invite_value(&v).is_err());
    }

    #[test]
    fn https_uri_roundtrip() {
        let (_ks, id) = create_keystore_v1("pw", None).unwrap();
        let pk = id.public_key_hex();
        let wire = build_connect_invite_wire_map("ghal-bol-chat", &pk, Some("Ada")).unwrap();
        let uri = connect_invite_https_uri_from_wire_map(&wire).unwrap();
        assert!(uri.starts_with("https://ghalbol.com/connect/"));
        assert!(uri.contains(&pk.to_lowercase()));
        assert!(uri.contains("alias=Ada"));
        let parsed = parse_connect_invite_uri(&uri).unwrap();
        verify_ghal_bol_connect_invite_value(&parsed).unwrap();
        assert_eq!(parsed["global_alias"], "Ada");
        assert_eq!(parsed["format_version"], CONNECT_INVITE_FORMAT_VERSION_V3);
    }

    #[test]
    fn app_uri_roundtrip() {
        let (_ks, id) = create_keystore_v1("pw", None).unwrap();
        let pk = id.public_key_hex();
        let wire = build_connect_invite_wire_map("ghal-bol-chat", &pk, None).unwrap();
        let uri = connect_invite_app_uri_from_wire_map(&wire).unwrap();
        assert!(uri.starts_with("ghalbol://connect/"));
        let parsed = parse_connect_invite_uri(&uri).unwrap();
        verify_ghal_bol_connect_invite_value(&parsed).unwrap();
    }

    #[test]
    fn ed25519_identity_https_uri_roundtrip() {
        use crate::identity::IdentityAlgorithm;
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Ed25519, None).unwrap();
        let wire_str = id.identity_wire();
        let wire = build_connect_invite_wire_map("ghal-bol-chat", &wire_str, Some("Ada")).unwrap();
        let uri = connect_invite_https_uri_from_wire_map(&wire).unwrap();
        assert!(uri.contains("ed25519%3A"));
        let parsed = parse_connect_invite_uri(&uri).unwrap();
        verify_ghal_bol_connect_invite_value(&parsed).unwrap();
        assert_eq!(parsed["identity_wire"].as_str().unwrap(), wire_str);
        assert_eq!(parsed["global_alias"], "Ada");
    }

    #[test]
    fn v3_wire_ok() {
        let (_ks, id) = create_keystore_v1("pw", None).unwrap();
        let wire = build_connect_invite_wire_map("ghal-bol-chat", &id.public_key_hex(), Some("Bob"))
            .unwrap();
        verify_ghal_bol_connect_invite_value(&wire).unwrap();
        assert_eq!(wire["format_version"], CONNECT_INVITE_FORMAT_VERSION_V3);
        assert_eq!(wire["global_alias"], "Bob");
    }
}
