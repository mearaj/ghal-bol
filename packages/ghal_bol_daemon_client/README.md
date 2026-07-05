# ghal_bol_daemon_client (Dart)

SDK for the Ghal Bol background daemon — JSON-RPC over Unix socket.

Used by `ghal_bol_ui` (reference integrator). Third-party apps should depend on this package directly.

See workspace `docs/DAEMON_INTEGRATOR.md`.

```dart
import 'package:ghal_bol_daemon_client/ghal_bol_daemon_client.dart';

final cfg = IntegratorConfig(
  appNamespace: 'com.example.chat',
  xdgRuntimeDir: Platform.environment['XDG_RUNTIME_DIR'],
);
// After spawning ghal_bol_daemon with cfg.daemonSpawnEnv():
final client = await DaemonClient.connect(cfg, socketPath: cfg.socketPath);
await client.unlock('password');
```
