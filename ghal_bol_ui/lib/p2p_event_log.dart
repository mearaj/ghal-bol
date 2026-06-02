import "app_log.dart";
import "public_key_hex.dart";
import "user_flow_log.dart";

/// Dedupes first connect/ready per peer; cleared on disconnect so reconnects are visible.
final Set<String> _p2pOncePerPeer = {};

void _clearPeerDedup(String peerId) {
  final p = peerId.trim();
  if (p.isEmpty) return;
  _p2pOncePerPeer.remove("connected:$p");
  _p2pOncePerPeer.remove("identified:$p");
  _p2pOncePerPeer.remove("ready:$p");
}

void logP2pEvent(Map<String, dynamic> ev) {
  final kind = ev["kind"]?.toString() ?? "?";
  switch (kind) {
    case "native_log":
      _logNativeService(ev);
      return;
    case "listening":
      P2pFlowLog.step("listening", {"addr": ev["multiaddr"]?.toString() ?? "?"});
      return;
    case "node_ready":
      P2pFlowLog.step("node_ready_wire");
      return;
    case "node_stopped":
      final err = ev["error"]?.toString();
      if (err != null && err.isNotEmpty) {
        AppLog.instance.e("P2P", "node_stopped: $err");
      } else {
        AppLog.instance.i("P2P", "node_stopped");
      }
      return;
    case "peer_connected":
      final pid = _peerKeyFromEvent(ev);
      final firstConnect = _p2pOncePerPeer.add("connected:$pid");
      if (firstConnect) {
        P2pFlowLog.step("peer_connected", {"peer": pid});
      } else {
        P2pFlowLog.detail("peer_connected", "reconnect peer=$pid");
      }
      AppLog.instance.trace("peer_connected", "libp2p link up peer=$pid");
      return;
    case "peer_disconnected":
      final discPid = _peerKeyFromEvent(ev);
      _clearPeerDedup(discPid);
      P2pFlowLog.step("peer_disconnected", {"peer": discPid});
      AppLog.instance.trace("peer_disconnected", "dm stream down peer=$discPid");
      return;
    case "peer_identified":
      final pid = ev["peer_id"]?.toString() ?? "";
      if (!_p2pOncePerPeer.add("identified:$pid")) return;
      AppLog.instance.json("P2P", "peer_identified", {
        "peer_id": pid,
        "public_key_hex": _shortHex(
          ev["public_key_hex"],
        ),
      });
      return;
    case "chat_ready":
      final pid = _peerKeyFromEvent(ev);
      final firstReady = _p2pOncePerPeer.add("ready:$pid");
      P2pFlowLog.step("chat_ready", {
        "peer": pid,
        "first": firstReady.toString(),
      });
      AppLog.instance.trace("chat_ready", "DM stream writer open peer=$pid");
      return;
    case "dial_failed":
      P2pFlowLog.issue(
        "dial_failed",
        detail: "${ev["peer"] ?? ev["peer_id"]} ${ev["error"]}",
        check: "mDNS same LAN; or P2P/Coord dial_start",
      );
      return;
    case "send_failed":
      AppLog.instance.e(
        "P2P",
        "send_failed msg_id=${ev["message_id"]} error=${ev["error"]}",
      );
      return;
    case "outbound_sent":
      AppLog.instance.flow("P2P", "outbound_sent msg_id=${ev["message_id"]}");
      AppLog.instance.trace("outbound_sent", "msg_id=${ev["message_id"]} peer=${ev["peer_id"]}");
      return;
    case "call_signal":
      if (AppLog.logCallFlow) {
        AppLog.instance.callStep("Call/P2P", "wire_rx", {
          "signal": ev["signal"]?.toString() ?? "?",
          "call_id": ev["call_id"]?.toString() ?? "?",
          "from": _shortHex(ev["sender_public_key_hex"]) ?? "?",
        });
      }
      return;
    case "dm_message":
      final mk = ev["msg_kind"]?.toString() ?? "";
      if (mk == "text") {
        final text = ev["text"]?.toString() ?? "";
        final preview = text.length > 120 ? "${text.substring(0, 120)}…" : text;
        AppLog.instance.flowJson("P2P", "dm_message text (wire)", {
          "from": ev["from"],
          "id": ev["id"],
          "sender_public_key_hex": _shortHex(
            ev["sender_public_key_hex"],
          ),
          "text_preview": preview,
          "created_at_ms": ev["created_at_ms"],
        });
        if (ev["stores_updated"] == true) {
          AppLog.instance.flow("DM/store", "native persisted inbound to contacts+transcript");
        } else {
          AppLog.instance.w(
            "DM/store",
            "inbound text poll event but stores_updated=false (handler context?)",
          );
        }
        AppLog.instance.trace(
          "inbound_text",
          "id=${ev["id"]} from=${ev["from"]} stores=${ev["stores_updated"] == true}",
        );
      } else if (mk == "ack_received" || mk == "ack_read") {
        AppLog.instance.flow("P2P", "dm_message $mk from=${ev["from"]} ref=${ev["ref_id"]}");
        if (ev["stores_updated"] == true) {
          AppLog.instance.flow("DM/store", "native applied ack to transcript");
        } else {
          AppLog.instance.w(
            "DM/store",
            "ack poll event stores_updated=false kind=$mk ref=${ev["ref_id"]}",
          );
        }
        AppLog.instance.trace("inbound_$mk", "ref=${ev["ref_id"]} from=${ev["from"]}");
      }
      return;
    case "stores_updated":
      AppLog.instance.flow(
        "DM/store",
        "stores_updated (poll) msg_kind=${ev["msg_kind"]} from=${ev["from"]}",
      );
      return;
    case "error":
      AppLog.instance.e("P2P", ev["message"]?.toString() ?? "native error", ev);
      return;
    default:
      if (AppLog.logNativeDebug) {
        AppLog.instance.d("P2P", kind);
      }
  }
}

String _peerKeyFromEvent(Map<String, dynamic> ev) {
  final pk = publicKeyHexFromEvent(ev);
  if (pk.isNotEmpty) return pk;
  return ev["peer_id"]?.toString().trim() ?? "";
}

String? _shortHex(Object? v) {
  final s = v?.toString().trim() ?? "";
  if (s.length <= 16) return s.isEmpty ? null : s;
  return "${s.substring(0, 8)}…${s.substring(s.length - 8)}";
}

/// libp2p worker inside `libghal_bol` (Rust `native_log` → FFI).
void _logNativeService(Map<String, dynamic> ev) {
  final tag = ev["tag"]?.toString() ?? "libp2p";
  final msg = ev["message"]?.toString() ?? "";
  final level = (ev["level"]?.toString() ?? "info").toLowerCase();
  final isConnectivityTag = tag == "flow"
      || tag == "net"
      || tag == "p2p"
      || tag == "swarm"
      || tag == "kad"
      || tag == "listen"
      || tag == "relay"
      || tag == "coord"
      || tag == "mdns"
      || tag == "dial"
      || tag == "autonat"
      || tag == "dcutr"
      || tag == "upnp"
      || tag == "stream";
  final isDmTag = tag == "DM/store"
      || tag == "Contacts"
      || tag == "Transcript"
      || tag == "outbound"
      || tag == "outbox"
      || tag == "delivery_ack"
      || tag == "read_ack"
      || tag == "session"
      || isConnectivityTag;
  if (level == "debug" && !AppLog.logNativeDebug && !isConnectivityTag && !isDmTag) {
    return;
  }
  // Drop noisy duplicate native lines when Dart already logs the FFI event.
  final logTag = "Native/$tag";
  switch (level) {
    case "debug":
      if (isConnectivityTag && AppLog.logP2pFlow) {
        AppLog.instance.i(logTag, msg);
      } else if (isDmTag && AppLog.logMessageFlow) {
        AppLog.instance.flow(logTag, msg);
      } else if (AppLog.logNativeDebug) {
        AppLog.instance.d(logTag, msg);
      }
      break;
    case "warn":
    case "warning":
      AppLog.instance.w(logTag, msg);
      break;
    case "error":
      AppLog.instance.e(logTag, msg);
      break;
    default:
      if (isConnectivityTag && AppLog.logP2pFlow) {
        AppLog.instance.i(logTag, msg);
      } else if (isDmTag && AppLog.logMessageFlow) {
        AppLog.instance.flow(logTag, msg);
      } else {
        AppLog.instance.i(logTag, msg);
      }
  }
}
