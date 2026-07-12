import "dart:async";
import "dart:io";

import "package:flutter/material.dart";
import "package:path_provider/path_provider.dart";
import "package:share_plus/share_plus.dart";

import "app_log.dart";

/// Tags / areas used only for the on-screen list filter (not logging toggles).
enum _ShowArea { calls, account, p2p, messages }

/// Scrollable session log (More → App log).
class AppLogScreen extends StatefulWidget {
  const AppLogScreen({super.key});

  @override
  State<AppLogScreen> createState() => _AppLogScreenState();
}

class _AppLogScreenState extends State<AppLogScreen> {
  final _searchCtrl = TextEditingController();
  final _scroll = ScrollController();
  AppLogLevel? _minLevel;
  bool _followTail = true;
  final Set<_ShowArea> _showAreas = {};
  bool _problemsOnly = false;
  bool _recordExpanded = false;
  bool _exportBusy = false;
  int _exportSeq = 0;

  bool _verboseNative = AppLog.logNativeDebug;
  bool _messageFlow = AppLog.logMessageFlow;
  bool _userFlow = AppLog.logUserFlow;
  bool _callFlow = AppLog.logCallFlow;
  bool _sessionFlow = AppLog.logSessionFlow;
  bool _p2pFlow = AppLog.logP2pFlow;

  @override
  void initState() {
    super.initState();
    AppLog.instance.addListener(_onLog);
    _scroll.addListener(_onScroll);
  }

  @override
  void dispose() {
    AppLog.instance.removeListener(_onLog);
    _scroll.removeListener(_onScroll);
    _searchCtrl.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _onLog() {
    if (!mounted) return;
    setState(() {});
    if (!_followTail) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_scroll.hasClients) return;
      if (AppLog.instance.entries.isEmpty) {
        _scroll.jumpTo(0);
        return;
      }
      _jumpToEnd();
    });
  }

  void _onScroll() {
    if (!_scroll.hasClients) return;
    final atEnd = _scroll.position.pixels >= _scroll.position.maxScrollExtent - 48;
    if (_followTail != atEnd) setState(() => _followTail = atEnd);
  }

  void _jumpToEnd() {
    if (!_scroll.hasClients) return;
    final max = _scroll.position.maxScrollExtent;
    if (!max.isFinite) return;
    _scroll.jumpTo(max.clamp(0.0, max));
  }

  void _resetScrollToTop() {
    if (!_scroll.hasClients) return;
    _scroll.jumpTo(0);
  }

  bool _entryInArea(AppLogEntry e, _ShowArea area) {
    final tag = e.tag;
    switch (area) {
      case _ShowArea.calls:
        return tag.startsWith("Call");
      case _ShowArea.account:
        return tag == "Session" || tag == "Daemon" || tag.startsWith("Session/");
      case _ShowArea.p2p:
        return tag.startsWith("P2P");
      case _ShowArea.messages:
        return tag == "Trace" ||
            tag == "Chat" ||
            tag == "Hub" ||
            tag == "Contacts" ||
            tag == "Retry" ||
            tag.startsWith("DM/");
    }
  }

  bool _isProblem(AppLogEntry e) =>
      e.level.index >= AppLogLevel.warn.index || e.message.contains("issue=");

  List<AppLogEntry> get _visible {
    var list = AppLog.instance.filtered(
      minLevel: _minLevel,
      search: _searchCtrl.text.trim(),
    );
    if (_showAreas.isNotEmpty) {
      list = list.where((e) => _showAreas.any((a) => _entryInArea(e, a))).toList();
    }
    if (_problemsOnly) {
      list = list.where(_isProblem).toList();
    }
    return list;
  }

  void _toggleArea(_ShowArea area) {
    setState(() {
      if (_showAreas.contains(area)) {
        _showAreas.remove(area);
      } else {
        _showAreas.add(area);
      }
    });
  }

  String _logFileName() {
    _exportSeq++;
    return "ghal_bol-app-log-${DateTime.now().millisecondsSinceEpoch}-$_exportSeq.txt";
  }

  /// Stream lines to disk — never build one giant `join` string (OOM on desktop/Android).
  Future<String?> _writeVisibleLogFile({
    required String directoryPath,
    required String action,
  }) async {
    final visible = List<AppLogEntry>.from(_visible);
    if (visible.isEmpty) return null;
    final path = "$directoryPath/${_logFileName()}";
    final file = File(path);
    IOSink? sink;
    try {
      sink = file.openWrite();
      sink.writeln(
        "### Ghal Bol log $action ${DateTime.now().toIso8601String()} "
        "visible=${visible.length} total=${AppLog.instance.entries.length}",
      );
      var n = 0;
      for (final e in visible) {
        sink.writeln(e.formatLine());
        n++;
        // Yield so the UI isolate can breathe; avoids freeze/OOM during big exports.
        if (n % 48 == 0) {
          await sink.flush();
          await Future<void>.delayed(Duration.zero);
        }
      }
      await sink.flush();
    } catch (e, st) {
      AppLog.instance.e("AppLog", "export_write_failed action=$action", e, st);
      try {
        if (await file.exists()) await file.delete();
      } catch (_) {}
      return null;
    } finally {
      await sink?.close();
    }
    if (!await file.exists() || await file.length() == 0) return null;
    return path;
  }

  Future<Directory> _downloadDirectory() async {
    final downloads = await getDownloadsDirectory();
    if (downloads != null) return downloads;
    if (Platform.isAndroid) {
      final ext = await getExternalStorageDirectory();
      if (ext != null) {
        final legacy = Directory("${ext.path}/Download");
        if (!await legacy.exists()) {
          await legacy.create(recursive: true);
        }
        return legacy;
      }
    }
    return await getApplicationDocumentsDirectory();
  }

  void _snack(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(message), duration: const Duration(seconds: 5)),
    );
  }

  Future<T?> _runExport<T>({
    required String action,
    required Future<T?> Function() run,
  }) async {
    if (_exportBusy) return null;
    if (_visible.isEmpty) {
      _snack("Nothing to save — widen filters or use the app to generate log lines.");
      return null;
    }
    setState(() => _exportBusy = true);
    AppLog.instance.removeListener(_onLog);
    AppLog.instance.i(
      "AppLog",
      "export_start action=$action visible=${_visible.length} "
      "total=${AppLog.instance.entries.length}",
    );
    try {
      return await run();
    } catch (e, st) {
      AppLog.instance.e("AppLog", "export_failed action=$action", e, st);
      if (mounted) _snack("$action failed: $e");
      return null;
    } finally {
      AppLog.instance.addListener(_onLog);
      if (mounted) setState(() => _exportBusy = false);
    }
  }

  /// Save filtered log lines to the device Downloads folder (or app documents fallback).
  Future<void> _downloadAll() async {
    await _runExport<void>(
      action: "download",
      run: () async {
        final dir = await _downloadDirectory();
        final path = await _writeVisibleLogFile(
          directoryPath: dir.path,
          action: "download",
        );
        if (!mounted) return;
        if (path == null) {
          AppLog.instance.w("AppLog", "export_done action=download ok=false");
          _snack("Save failed — no file written.");
          return;
        }
        AppLog.instance.i("AppLog", "export_done action=download ok=true path=$path");
        _snack("Saved to $path");
      },
    );
  }

  Future<void> _shareAll() async {
    await _runExport<void>(
      action: "share",
      run: () async {
        final dir = await getTemporaryDirectory();
        final path = await _writeVisibleLogFile(
          directoryPath: dir.path,
          action: "share",
        );
        if (!mounted) return;
        if (path == null) {
          AppLog.instance.w("AppLog", "export_done action=share ok=false");
          _snack("Share failed — no file written.");
          return;
        }
        AppLog.instance.i("AppLog", "export_done action=share ok=true path=$path");
        await SharePlus.instance.share(
          ShareParams(
            files: [XFile(path, mimeType: "text/plain")],
            subject: "Ghal Bol app log",
          ),
        );
      },
    );
  }

  Future<void> _confirmClear() async {
    final go = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text("Clear log?"),
        content: const Text("Removes all in-memory log lines for this session."),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Cancel")),
          FilledButton(onPressed: () => Navigator.pop(ctx, true), child: const Text("Clear")),
        ],
      ),
    );
    if (go != true || !mounted) return;
    AppLog.instance.removeListener(_onLog);
    AppLog.instance.clear();
    AppLog.instance.addListener(_onLog);
    if (!mounted) return;
    _resetScrollToTop();
    setState(() => _followTail = true);
  }

  @override
  Widget build(BuildContext context) {
    final lines = _visible;
    final cs = Theme.of(context).colorScheme;
    final textTheme = Theme.of(context).textTheme;
    final showFilterActive = _showAreas.isNotEmpty || _problemsOnly;

    return Scaffold(
      appBar: AppBar(
        title: const Text("App log"),
        actions: [
          IconButton(
            tooltip: _followTail ? "Following new lines" : "Scroll to latest",
            onPressed: () {
              setState(() => _followTail = true);
              _jumpToEnd();
            },
            icon: Icon(_followTail ? Icons.vertical_align_bottom : Icons.arrow_downward),
          ),
          IconButton(
            tooltip: "Download visible lines to Downloads",
            onPressed: lines.isEmpty || _exportBusy ? null : _downloadAll,
            icon: const Icon(Icons.download_outlined),
          ),
          IconButton(
            tooltip: "Share visible lines as a text file",
            onPressed: lines.isEmpty || _exportBusy ? null : _shareAll,
            icon: const Icon(Icons.share_outlined),
          ),
          IconButton(
            tooltip: "Clear log",
            onPressed: AppLog.instance.entries.isEmpty ? null : _confirmClear,
            icon: const Icon(Icons.delete_outline),
          ),
        ],
      ),
      body: Column(
        children: [
          Material(
            color: cs.surfaceContainerHighest.withValues(alpha: 0.35),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(12, 8, 12, 8),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text("Show in list", style: textTheme.labelLarge),
                  const SizedBox(height: 4),
                  Text(
                    "Pick none or several areas. Search below is separate.",
                    style: textTheme.bodySmall?.copyWith(color: cs.onSurfaceVariant),
                  ),
                  const SizedBox(height: 6),
                  Wrap(
                    spacing: 6,
                    runSpacing: 4,
                    children: [
                      FilterChip(
                        label: const Text("Calls"),
                        selected: _showAreas.contains(_ShowArea.calls),
                        onSelected: (_) => _toggleArea(_ShowArea.calls),
                      ),
                      FilterChip(
                        label: const Text("Account"),
                        selected: _showAreas.contains(_ShowArea.account),
                        onSelected: (_) => _toggleArea(_ShowArea.account),
                      ),
                      FilterChip(
                        label: const Text("P2P"),
                        selected: _showAreas.contains(_ShowArea.p2p),
                        onSelected: (_) => _toggleArea(_ShowArea.p2p),
                      ),
                      FilterChip(
                        label: const Text("Messages"),
                        selected: _showAreas.contains(_ShowArea.messages),
                        onSelected: (_) => _toggleArea(_ShowArea.messages),
                      ),
                      FilterChip(
                        label: const Text("Problems"),
                        selected: _problemsOnly,
                        onSelected: (v) => setState(() => _problemsOnly = v),
                      ),
                      if (showFilterActive)
                        ActionChip(
                          label: const Text("Clear"),
                          onPressed: () => setState(() {
                            _showAreas.clear();
                            _problemsOnly = false;
                          }),
                        ),
                    ],
                  ),
                  const SizedBox(height: 10),
                  TextField(
                    controller: _searchCtrl,
                    decoration: const InputDecoration(
                      hintText: "Search text in lines…",
                      prefixIcon: Icon(Icons.search, size: 20),
                      isDense: true,
                      border: OutlineInputBorder(),
                    ),
                    onChanged: (_) => setState(() {}),
                  ),
                  const SizedBox(height: 8),
                  Text("Minimum level", style: textTheme.labelLarge),
                  const SizedBox(height: 4),
                  SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: Row(
                      children: [
                        FilterChip(
                          label: const Text("All levels"),
                          selected: _minLevel == null,
                          onSelected: (_) => setState(() => _minLevel = null),
                        ),
                        const SizedBox(width: 6),
                        for (final lv in AppLogLevel.values)
                          Padding(
                            padding: const EdgeInsets.only(right: 6),
                            child: FilterChip(
                              label: Text(lv.label),
                              selected: _minLevel == lv,
                              onSelected: (_) => setState(() => _minLevel = lv),
                            ),
                          ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 4),
                  ExpansionTile(
                    tilePadding: EdgeInsets.zero,
                    initiallyExpanded: _recordExpanded,
                    onExpansionChanged: (v) => setState(() => _recordExpanded = v),
                    title: Text("Record new lines", style: textTheme.labelLarge),
                    subtitle: Text(
                      "Off = less noise while using the app; existing lines stay visible",
                      style: textTheme.bodySmall?.copyWith(color: cs.onSurfaceVariant),
                    ),
                    children: [
                      SwitchListTile(
                        contentPadding: EdgeInsets.zero,
                        title: const Text("Journey steps"),
                        subtitle: const Text("Numbered step= lines (master)"),
                        value: _userFlow,
                        onChanged: (v) => setState(() {
                          _userFlow = v;
                          AppLog.logUserFlow = v;
                        }),
                      ),
                      SwitchListTile(
                        contentPadding: EdgeInsets.zero,
                        title: const Text("Calls"),
                        subtitle: const Text("[Call] voice / video — independent of Journey"),
                        value: _callFlow,
                        onChanged: (v) => setState(() {
                          _callFlow = v;
                          AppLog.logCallFlow = v;
                        }),
                      ),
                      SwitchListTile(
                        contentPadding: EdgeInsets.zero,
                        title: const Text("Account"),
                        subtitle: const Text("[Session] [Daemon] unlock / login"),
                        value: _sessionFlow,
                        onChanged: _userFlow
                            ? (v) => setState(() {
                                  _sessionFlow = v;
                                  AppLog.logSessionFlow = v;
                                })
                            : null,
                      ),
                      SwitchListTile(
                        contentPadding: EdgeInsets.zero,
                        title: const Text("P2P network"),
                        subtitle: const Text("[P2P] connect / dial / streams"),
                        value: _p2pFlow,
                        onChanged: _userFlow
                            ? (v) => setState(() {
                                  _p2pFlow = v;
                                  AppLog.logP2pFlow = v;
                                })
                            : null,
                      ),
                      SwitchListTile(
                        contentPadding: EdgeInsets.zero,
                        title: const Text("Chat & delivery"),
                        subtitle: const Text("[Trace] [Chat] [Hub] DM ticks"),
                        value: _messageFlow,
                        onChanged: (v) => setState(() {
                          _messageFlow = v;
                          AppLog.logMessageFlow = v;
                        }),
                      ),
                      SwitchListTile(
                        contentPadding: EdgeInsets.zero,
                        title: const Text("Native RPC debug"),
                        subtitle: const Text("Verbose daemon / FFI"),
                        value: _verboseNative,
                        onChanged: (v) => setState(() {
                          _verboseNative = v;
                          AppLog.logNativeDebug = v;
                        }),
                      ),
                    ],
                  ),
                  const SizedBox(height: 4),
                  Text(
                    "${lines.length} / ${AppLog.instance.entries.length} lines · "
                    "uptime ${AppLog.instance.sessionUptimeLabel()}",
                    style: textTheme.bodySmall,
                  ),
                ],
              ),
            ),
          ),
          Expanded(
            child: lines.isEmpty
                ? Center(
                    child: Text(
                      AppLog.instance.entries.isEmpty
                          ? "No log lines yet. Use the app and events will appear here."
                          : "No lines match the current filters.",
                      textAlign: TextAlign.center,
                      style: TextStyle(color: cs.onSurfaceVariant),
                    ),
                  )
                : Scrollbar(
                    thumbVisibility: true,
                    controller: _scroll,
                    child: ListView.builder(
                      controller: _scroll,
                      padding: const EdgeInsets.fromLTRB(8, 4, 8, 16),
                      itemCount: lines.length,
                      itemBuilder: (context, i) {
                        final e = lines[i];
                        return _LogLineTile(entry: e);
                      },
                    ),
                  ),
          ),
        ],
      ),
    );
  }
}

class _LogLineTile extends StatelessWidget {
  const _LogLineTile({required this.entry});

  final AppLogEntry entry;

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Text(
        entry.formatLine(),
        style: TextStyle(
          fontFamily: "monospace",
          fontSize: 11.5,
          height: 1.35,
          color: cs.onSurface,
        ),
      ),
    );
  }
}
