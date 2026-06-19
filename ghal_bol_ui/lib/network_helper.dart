import "dart:async";

import "package:flutter/foundation.dart";
import "package:ghal_bol_ui/app_log.dart";

import "src/network_helper_platform_stub.dart"
    if (dart.library.io) "src/network_helper_platform_io.dart" as platform;

/// OS default-route network truth for **UI display only** (offline hints).
///
/// P2P handover, coord, and ack policy stay in Rust `:p2p` / daemon — never call
/// `p2p_notify_network_change` from here. See `docs/TRANSPORT.md` § Network truth.
class GhalBolNetworkSnapshot {
  const GhalBolNetworkSnapshot({
    required this.defaultTransport,
    required this.internetValidated,
    required this.hasInternet,
    required this.wifiLinkUp,
    this.defaultRouteIface,
    required this.hasLiveSnapshot,
    this.source,
  });

  factory GhalBolNetworkSnapshot.unknown() => const GhalBolNetworkSnapshot(
    defaultTransport: "none",
    internetValidated: false,
    hasInternet: false,
    wifiLinkUp: false,
    hasLiveSnapshot: false,
  );

  factory GhalBolNetworkSnapshot.fromJson(Map<String, dynamic> json) {
    if (json["ok"] != true) {
      return GhalBolNetworkSnapshot.unknown();
    }
    return GhalBolNetworkSnapshot(
      defaultTransport: json["default_transport"]?.toString() ?? "none",
      internetValidated: json["internet_validated"] == true,
      hasInternet: json["has_internet"] == true,
      wifiLinkUp: json["wifi_link_up"] == true,
      defaultRouteIface: json["default_route_iface"]?.toString(),
      hasLiveSnapshot: true,
      source: json["source"]?.toString(),
    );
  }

  final String defaultTransport;
  final bool internetValidated;
  final bool hasInternet;
  final bool wifiLinkUp;
  final String? defaultRouteIface;
  final bool hasLiveSnapshot;
  final String? source;

  bool get onMobileData => defaultTransport == "cell";
  bool get onLanPath =>
      defaultTransport == "wifi" || defaultTransport == "ethernet";

  /// Offline for user messaging — OS reports no internet capability.
  /// Brief `unvalidated` after Wi‑Fi associate is normal (TRANSPORT.md rule 5) — do not treat as offline.
  bool get appearsOffline => hasLiveSnapshot && !hasInternet;

  String get flowLabel {
    final validated = internetValidated ? "validated" : "unvalidated";
    final wifi = wifiLinkUp ? "wifi_up" : "wifi_down";
    final route = defaultRouteIface?.isNotEmpty == true
        ? " route=$defaultRouteIface"
        : "";
    return "os=$defaultTransport/$validated/$wifi$route";
  }

  @override
  bool operator ==(Object other) =>
      other is GhalBolNetworkSnapshot &&
      defaultTransport == other.defaultTransport &&
      internetValidated == other.internetValidated &&
      hasInternet == other.hasInternet &&
      wifiLinkUp == other.wifiLinkUp &&
      defaultRouteIface == other.defaultRouteIface &&
      hasLiveSnapshot == other.hasLiveSnapshot;

  @override
  int get hashCode => Object.hash(
    defaultTransport,
    internetValidated,
    hasInternet,
    wifiLinkUp,
    defaultRouteIface,
    hasLiveSnapshot,
  );
}

/// Singleton OS network probe for Flutter UI.
class NetworkHelper {
  NetworkHelper._();

  static final NetworkHelper instance = NetworkHelper._();

  static const _pollInterval = Duration(seconds: 1);
  static const _heartbeatInterval = Duration(seconds: 30);

  final ValueNotifier<GhalBolNetworkSnapshot> snapshot =
      ValueNotifier(GhalBolNetworkSnapshot.unknown());

  Timer? _pollTimer;
  Timer? _heartbeatTimer;
  bool _started = false;

  Future<void> start() async {
    if (_started) return;
    _started = true;
    AppLog.instance.i(
      "Network",
      "helper start platform=${platform.NetworkHelperPlatform.platformLabel}",
    );
    await _refresh(source: "start");
    _pollTimer = Timer.periodic(_pollInterval, (_) {
      unawaited(_refresh(source: "poll"));
    });
    _heartbeatTimer = Timer.periodic(_heartbeatInterval, (_) {
      _logHeartbeat();
    });
  }

  Future<void> stop() async {
    if (!_started) return;
    _started = false;
    _pollTimer?.cancel();
    _pollTimer = null;
    _heartbeatTimer?.cancel();
    _heartbeatTimer = null;
    snapshot.value = GhalBolNetworkSnapshot.unknown();
    AppLog.instance.i("Network", "helper stop");
  }

  void _logHeartbeat() {
    final s = snapshot.value;
    if (!s.hasLiveSnapshot) {
      AppLog.instance.d("Network", "heartbeat waiting for first probe");
      return;
    }
    AppLog.instance.i(
      "Network",
      "heartbeat ${s.flowLabel} internet=${s.hasInternet} src=${s.source ?? "?"}",
    );
  }

  Future<void> _refresh({required String source}) async {
    try {
      final json = await platform.NetworkHelperPlatform.fetchSnapshot();
      if (json == null) return;
      _applySnapshot(
        GhalBolNetworkSnapshot.fromJson(json),
        source: json["source"]?.toString() ?? source,
      );
    } catch (e) {
      AppLog.instance.w("Network", "probe failed source=$source err=$e");
    }
  }

  void _applySnapshot(GhalBolNetworkSnapshot next, {required String source}) {
    if (!next.hasLiveSnapshot) {
      AppLog.instance.w("Network", "probe not ok source=$source");
      return;
    }
    final prev = snapshot.value;
    if (prev == next) return;
    snapshot.value = next;
    AppLog.instance.flow(
      "Network",
      "change source=$source ${next.flowLabel} internet=${next.hasInternet}",
    );
  }
}
