//! **`ghal_bol_connect_v1`** connect invite — **format 2 only** (`public_key_hex` on the wire).
//!
//! **Invite URLs (current):**
//! - `https://ghalbol.com/connect/<public_key_hex>`
//! - `ghalbol://connect/<public_key_hex>`
//!
//! Optional query: `?alias=<display name>` (URL-encoded).

use serde_json::Value;

use crate::public_key_util::secp256k1_public_key_from_hex;

const SHARE: &str = "ghal_bol_connect_v1";
/// Sole wire format for new QR / links.
pub const CONNECT_INVITE_FORMAT_VERSION: u64 = 2;

/// HTTPS invite host (owned domain).
pub const CONNECT_HTTPS_HOST: &str = "ghalbol.com";
/// App deep-link scheme.
pub const CONNECT_APP_SCHEME: &str = "ghalbol";
/// Path segment after host or scheme: `/connect/<public_key_hex>`.
pub const CONNECT_PATH_SEGMENT: &str = "connect";

const DEFAULT_TOPIC: &str = "ghal-bol-chat";

fn hex66_from_field(label: &str, hex_s: &str) -> Result<(), String> {
    let s = hex_s.trim();
    if s.len() != 66 {
        return Err(format!("{label}: expected 66 hex chars (compressed secp256k1)"));
    }
    hex::decode(s).map_err(|e| format!("{label}: hex: {e}"))?;
    Ok(())
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
    hex66_from_field("public_key_hex", pk_hex)?;
    let _ = secp256k1_public_key_from_hex(pk_hex)?;
    if field_nonempty_string(v, "encryption_public_key_hex") {
        return Err("encryption_public_key_hex is not used (single secp256k1 key)".to_string());
    }
    if field_nonempty_string(v, "coord_base_url") {
        return Err("coord_base_url must not appear on invite wire (use app env / preferences)".to_string());
    }
    Ok(())
}

/// Build format-2 wire map (`public_key_hex` only).
pub fn build_connect_invite_wire_map(
    topic: &str,
    public_key_hex: &str,
    peer_alias: Option<&str>,
) -> Result<Value, String> {
    hex66_from_field("public_key_hex", public_key_hex)?;
    let pk = public_key_hex.trim().to_lowercase();
    let mut m = serde_json::json!({
        "ghalbol.share": SHARE,
        "format_version": CONNECT_INVITE_FORMAT_VERSION,
        "topic": topic,
        "public_key_hex": pk,
    });
    if let Some(a) = peer_alias {
        let t = a.trim();
        if !t.is_empty() {
            m["peer_alias"] = Value::String(t.to_string());
        }
    }
    Ok(m)
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

/// `https://ghalbol.com/connect/<public_key_hex>`
pub fn connect_invite_https_uri_from_wire_map(v: &Value) -> Result<String, String> {
    verify_ghal_bol_connect_invite_value(v)?;
    let pk = v
        .get("public_key_hex")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing public_key_hex".to_string())?
        .trim()
        .to_lowercase();
    let mut url = format!("https://{CONNECT_HTTPS_HOST}/{CONNECT_PATH_SEGMENT}/{pk}");
    if let Some(alias) = v.get("peer_alias").and_then(|x| x.as_str()) {
        let t = alias.trim();
        if !t.is_empty() {
            url.push('?');
            url.push_str(&format!("alias={}", percent_encode_query_component(t)));
        }
    }
    Ok(url)
}

/// `ghalbol://connect/<public_key_hex>`
pub fn connect_invite_app_uri_from_wire_map(v: &Value) -> Result<String, String> {
    verify_ghal_bol_connect_invite_value(v)?;
    let pk = v
        .get("public_key_hex")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "missing public_key_hex".to_string())?
        .trim()
        .to_lowercase();
    let mut url = format!("{CONNECT_APP_SCHEME}://{CONNECT_PATH_SEGMENT}/{pk}");
    if let Some(alias) = v.get("peer_alias").and_then(|x| x.as_str()) {
        let t = alias.trim();
        if !t.is_empty() {
            url.push('?');
            url.push_str(&format!("alias={}", percent_encode_query_component(t)));
        }
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
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
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
    let seg = seg.trim().trim_end_matches('/');
    hex66_from_field("public_key_hex", seg)?;
    Ok(seg.to_lowercase())
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

/// Verify **`ghal_bol_connect_v1`** JSON (format **2** only).
pub fn verify_ghal_bol_connect_invite_value(v: &Value) -> Result<(), String> {
    verify_v2(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1;

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
        let pid = id.to_libp2p_keypair().unwrap().public().to_peer_id().to_string();
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
        assert_eq!(parsed["peer_alias"], "Ada");
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

}
