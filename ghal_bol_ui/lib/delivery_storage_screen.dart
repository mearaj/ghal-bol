import "dart:async";

import "package:flutter/material.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";
import "package:ghal_bol_ui/app_env_config.dart";
import "package:ghal_bol_ui/p2p_link_error_ui.dart";

/// User-visible delivery mailbox: quota, pending metadata, TTL actions.
class DeliveryStorageScreen extends StatefulWidget {
  const DeliveryStorageScreen({super.key});

  @override
  State<DeliveryStorageScreen> createState() => _DeliveryStorageScreenState();
}

class _DeliveryStorageScreenState extends State<DeliveryStorageScreen> {
  Map<String, dynamic>? _status;
  List<dynamic> _rows = const [];
  String? _error;
  bool _loading = true;

  @override
  void initState() {
    super.initState();
    unawaited(_refresh());
  }

  Future<void> _refresh() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final status = await GhalBolP2p.deliveryConnectionStatus();
      final list = await GhalBolP2p.deliveryMailboxList(includeExpired: true);
      if (!mounted) return;
      setState(() {
        _status = status;
        _rows = (list["snapshot"]?["rows"] as List<dynamic>?) ?? const [];
        _loading = false;
        if (status["ok"] != true) {
          _error = _friendlyDeliveryError(status["error"]?.toString());
        } else if (list["ok"] != true) {
          _error = _friendlyDeliveryError(list["error"]?.toString());
        } else {
          final last = status["last_error"]?.toString();
          if (status["connected"] != true && last != null && last.isNotEmpty) {
            _error = _friendlyDeliveryError(last);
          }
        }
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = _friendlyDeliveryError(e.toString());
        _loading = false;
      });
    }
  }

  String? _friendlyDeliveryError(String? raw) {
    if (raw == null || raw.trim().isEmpty) return null;
    return networkAwareUserP2pError(raw) ??
        shortUserP2pError(raw) ??
        "Couldn't load message storage. Try again.";
  }

  String _formatBytes(num? n) {
    if (n == null) return "—";
    final b = n.toDouble();
    if (b < 1024) return "${b.toInt()} B";
    if (b < 1024 * 1024) return "${(b / 1024).toStringAsFixed(1)} KB";
    return "${(b / (1024 * 1024)).toStringAsFixed(1)} MB";
  }

  Future<void> _extendTtl(String messageId) async {
    final policy = _status?["policy"] as Map<String, dynamic>?;
    final defaultSecs = (policy?["default_ttl_secs"] as num?)?.toInt() ?? 604800;
    final r = await GhalBolP2p.deliveryExtendTtl(
      messageId: messageId,
      extendSecs: defaultSecs,
    );
    if (!mounted) return;
    if (r["ok"] != true) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            _friendlyDeliveryError(r["error"]?.toString()) ?? "Couldn't extend storage time.",
          ),
        ),
      );
    }
    await _refresh();
  }

  Future<void> _resend(String messageId) async {
    final r = await GhalBolP2p.deliveryResendMessage(messageId);
    if (!mounted) return;
    if (r["ok"] != true) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            _friendlyDeliveryError(r["error"]?.toString()) ?? "Couldn't resend message.",
          ),
        ),
      );
    }
    await _refresh();
  }

  @override
  Widget build(BuildContext context) {
    final quota = _status?["quota"] as Map<String, dynamic>?;
    final policy = _status?["policy"] as Map<String, dynamic>?;
    final connected = _status?["connected"] == true;
    final deliveryUrl = AppEnvConfig.get("GHAL_BOL_DELIVERY_URL") ?? "";

    return Scaffold(
      appBar: AppBar(
        title: const Text("Delivery storage"),
        actions: [
          IconButton(
            icon: const Icon(Icons.refresh),
            onPressed: _loading ? null : _refresh,
          ),
        ],
      ),
      body: _loading
          ? const Center(child: CircularProgressIndicator())
          : ListView(
              padding: const EdgeInsets.all(16),
              children: [
                if (_error != null)
                  Text(_error!, style: TextStyle(color: Theme.of(context).colorScheme.error)),
                ListTile(
                  title: Text(connected ? "Connected" : "Reconnecting…"),
                  subtitle: Text(
                    deliveryUrl.isEmpty
                        ? "Message storage is not configured for this build."
                        : "Secure message storage",
                  ),
                  leading: Icon(
                    connected ? Icons.cloud_done : Icons.cloud_off,
                    color: connected ? Colors.green : Colors.orange,
                  ),
                ),
                if (quota != null) ...[
                  const SizedBox(height: 8),
                  Text(
                    "Quota: ${_formatBytes(quota["used_bytes"] as num?)} / "
                    "${_formatBytes(quota["allocated_bytes"] as num?)} "
                    "(${quota["pending_count"] ?? 0} pending)",
                  ),
                  LinearProgressIndicator(
                    value: () {
                      final used = (quota["used_bytes"] as num?)?.toDouble() ?? 0;
                      final alloc = (quota["allocated_bytes"] as num?)?.toDouble() ?? 1;
                      if (alloc <= 0) return 0.0;
                      return (used / alloc).clamp(0.0, 1.0);
                    }(),
                  ),
                ],
                if (policy != null) ...[
                  const SizedBox(height: 12),
                  Text(
                    "TTL policy: min ${policy["min_ttl_secs"]}s · "
                    "default ${policy["default_ttl_secs"]}s · "
                    "max ${policy["max_ttl_secs"]}s",
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ],
                const Divider(height: 32),
                Text("Your messages on server", style: Theme.of(context).textTheme.titleMedium),
                const SizedBox(height: 8),
                if (_rows.isEmpty)
                  const Text("No pending or recent metadata on the delivery server.")
                else
                  ..._rows.map((raw) {
                    final row = raw as Map<String, dynamic>;
                    final id = row["message_id"]?.toString() ?? "";
                    final state = row["state"]?.toString() ?? "";
                    final bytes = row["size_bytes"];
                    final expires = row["expires_at_ms"];
                    return Card(
                      child: ListTile(
                        title: Text(id, maxLines: 1, overflow: TextOverflow.ellipsis),
                        subtitle: Text(
                          "$state · ${_formatBytes(bytes as num?)} · "
                          "expires $expires",
                        ),
                        trailing: state == "expired"
                            ? TextButton(
                                onPressed: () => _resend(id),
                                child: const Text("Resend"),
                              )
                            : state == "queued"
                                ? TextButton(
                                    onPressed: () => _extendTtl(id),
                                    child: const Text("Extend TTL"),
                                  )
                                : null,
                      ),
                    );
                  }),
              ],
            ),
    );
  }
}
