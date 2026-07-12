/// dart:ffi to the **`ghal_bol_core`** Rust `cdylib` (`ghal_bol_core_ffi_*` symbols).
///
/// **Architecture:** `ghal_bol_ui` should do **identity, crypto, persistence, and P2P** through this
/// surface (the **`lib_ghal_bol_core`** native library) whenever practical. The UI layer handles layout,
/// navigation, and platform glue only.
///
/// **Legitimate exceptions** (stay in Flutter / host code):
/// - **OS permissions** the engine must request (e.g. camera, notifications).
/// - **Android foreground service** lifecycle when the OS requires Kotlin / `MethodChannel` (still
///   the same process; native P2P remains in `lib_ghal_bol_core`).
/// - **Pure presentation** (themes, responsive shells, QR rendering, invite URI formatting for UX).
///
/// When adding storage or crypto, extend **`ghal_bol_core`** and expose new `ghal_bol_core_ffi_*` symbols
/// rather than introducing parallel Dart-side persistence unless there is a clear reason.
library;

export "src/ghal_bol_ffi_result.dart";
export "src/ghal_bol_ffi_stub.dart"
    if (dart.library.io) "src/ghal_bol_ffi_io.dart";
