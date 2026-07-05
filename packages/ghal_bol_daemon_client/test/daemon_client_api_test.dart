import "package:ghal_bol_daemon_client/ghal_bol_daemon_client.dart";
import "package:test/test.dart";

/// Must match `DaemonMethod::ALL` wire order in `ghal_bol/src/daemon/client_api.rs`.
const _rustMirrorWireNames = <String>[
  "ping",
  "unlock",
  "lock",
  "session_unlocked",
  "p2p_start",
  "p2p_stop",
  "p2p_is_running",
  "network_snapshot",
  "p2p_poll",
  "p2p_send_text_dm",
  "p2p_call_signal",
  "p2p_call_media",
  "p2p_call_status",
  "p2p_dismiss_incoming_call_alert",
  "p2p_force_end_active_call",
  "p2p_take_incoming_call_wake",
  "p2p_take_unlock_wake",
  "ui_session_prepare_reconnect",
  "ui_process_exiting",
  "p2p_transcript_load_merged",
  "p2p_call_video",
  "p2p_call_video_frame",
  "p2p_call_video_texture",
  "p2p_call_video_push_camera_frame",
  "p2p_requeue_outbound_dm",
  "p2p_send_ack_dm",
  "p2p_dial_bootstrap",
  "p2p_register_dm_peer",
  "p2p_set_app_ack_read_enabled",
  "p2p_set_app_ui_visible",
  "p2p_sync_ui_session",
  "p2p_nudge_read_catchup",
  "p2p_set_foreground_peer",
  "coord_set_base_url",
  "coord_lookup_peer",
  "coord_register_now",
];

void main() {
  test("DaemonMethod.all matches Rust client_api wire names", () {
    expect(DaemonMethod.all, _rustMirrorWireNames);
  });

  test("DaemonMethod wire names are unique", () {
    expect(DaemonMethod.all.toSet().length, DaemonMethod.all.length);
  });

  test("IntegratorConfig paths match Rust layout", () {
    final cfg = IntegratorConfig(
      appNamespace: "com.example.chat",
      xdgRuntimeDir: "/run/user/1000",
    );
    expect(cfg.socketPath, "/run/user/1000/ghalbol/com.example.chat/p2p.sock");
    expect(cfg.runtimeDir, "/run/user/1000/ghalbol/com.example.chat");
    expect(cfg.daemonSpawnEnv()["GHAL_BOL_APP_NAMESPACE"], "com.example.chat");
  });
}
