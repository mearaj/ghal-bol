//! Android `:p2p` process: JNI entry to run the Unix-socket daemon inside `libghal_bol.so`.

use std::path::Path;

use jni::errors::LogContextErrorAndDefault;
use jni::objects::{JClass, JObject, JString};
use jni::sys::{jboolean, JNI_FALSE, JNI_TRUE};
use jni::EnvUnowned;

use crate::c_ffi::configure_android_data_directory;
use crate::daemon::run_daemon;
use crate::p2p::notify_network_change;

/// Must run once per `:p2p` process before any `reqwest`/coord HTTPS (see rustls-platform-verifier Android docs).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ghalbol_P2pDaemonNative_initRustlsPlatformVerifier<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    context: JObject<'local>,
) {
    let _ = unowned_env
        .with_env(|env| -> jni::errors::Result<()> {
            rustls_platform_verifier::android::init_with_env(env, context)?;
            Ok(())
        })
        .resolve_with::<LogContextErrorAndDefault, _>(|| {
            "ghal_bol initRustlsPlatformVerifier".to_string()
        });
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ghalbol_P2pDaemonNative_configureDataDirectory<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) {
    let _ = unowned_env
        .with_env(|env| -> jni::errors::Result<()> {
            let p: String = path.try_to_string(env)?;
            configure_android_data_directory(p.trim());
            Ok(())
        })
        .resolve_with::<LogContextErrorAndDefault, _>(|| {
            "ghal_bol configureDataDirectory".to_string()
        });
}

/// Blocks the calling thread running the JSON-RPC listener (call from a background thread).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ghalbol_P2pDaemonNative_runDaemon<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    socket_path: JString<'local>,
) -> jboolean {
    unowned_env
        .with_env(|env| -> jni::errors::Result<jboolean> {
            let p: String = socket_path.try_to_string(env)?;
            let path = Path::new(p.trim());
            match run_daemon(path) {
                Ok(()) => Ok(JNI_TRUE),
                Err(e) => {
                    eprintln!("ghal_bol run_daemon failed: {e}");
                    Ok(JNI_FALSE)
                }
            }
        })
        .resolve_with::<LogContextErrorAndDefault, _>(|| "ghal_bol runDaemon".to_string())
}

/// Called from `ConnectivityManager` when the default network changes (`:p2p` process).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ghalbol_P2pDaemonNative_notifyNetworkChange<'local>(
    _unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    notify_network_change();
}
