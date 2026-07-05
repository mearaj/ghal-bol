import "daemon_client_api.dart";

/// Per-integrator configuration (mirrors Rust `ghal_bol::daemon::IntegratorConfig`).
class IntegratorConfig {
  IntegratorConfig({
    required this.appNamespace,
    this.socketPathOverride,
    this.runtimeDirOverride,
    this.xdgRuntimeDir,
  });

  final String appNamespace;
  final String? socketPathOverride;
  final String? runtimeDirOverride;
  final String? xdgRuntimeDir;

  String get sanitizedNamespace =>
      DaemonIntegratorConfig.sanitizeAppNamespaceSegment(appNamespace);

  String get runtimeDir =>
      runtimeDirOverride ??
      DaemonIntegratorConfig.runtimeDirForAppNamespace(
        appNamespace,
        xdgRuntimeDir: xdgRuntimeDir,
      );

  String get socketPath =>
      socketPathOverride ??
      DaemonIntegratorConfig.socketPathForAppNamespace(
        appNamespace,
        xdgRuntimeDir: xdgRuntimeDir,
      );

  String get uiPresencePath =>
      DaemonIntegratorConfig.uiPresencePathForAppNamespace(
        appNamespace,
        xdgRuntimeDir: xdgRuntimeDir,
      );

  /// Environment for spawning `ghal_bol_daemon` (Linux).
  Map<String, String> daemonSpawnEnv() => {
        "GHAL_BOL_APP_NAMESPACE": appNamespace,
        if (socketPathOverride != null)
          "GHAL_BOL_DAEMON_SOCKET": socketPathOverride!,
        if (runtimeDirOverride != null)
          "GHAL_BOL_RUNTIME_DIR": runtimeDirOverride!,
      };
}
