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

/// Set exactly once per process: `ndk_context::initialize_android_context` asserts the context
/// was never set before (panics on a second call). The `:p2p` `onStartCommand` runs more than
/// once (daemon-start + listener-start + call re-promote), so guard against re-entry here.
static ANDROID_AUDIO_INITED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Hand cpal/Oboe the JavaVM + Android Context so native voice can open the mic /
/// speaker in the `:p2p` process. Safe to call repeatedly; only the first call wins.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ghalbol_P2pDaemonNative_initAndroidAudio<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    context: JObject<'local>,
) {
    // Claim the one-shot slot before touching `ndk_context`/leaking a global ref.
    if ANDROID_AUDIO_INITED
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return;
    }
    let _ = unowned_env
        .with_env(|env| -> jni::errors::Result<()> {
            crate::android_jni_cache::cache_daemon_native_class(env, &_class)?;
            let vm = env.get_java_vm()?;
            // `into_raw` keeps the global ref alive for the process lifetime (never dropped).
            let global = env.new_global_ref(context)?;
            let vm_ptr = vm.get_raw() as *mut std::ffi::c_void;
            let ctx_ptr = global.into_raw() as *mut std::ffi::c_void;
            unsafe {
                ndk_context::initialize_android_context(vm_ptr, ctx_ptr);
            }
            crate::call_media::set_android_audio_ready();
            Ok(())
        })
        .resolve_with::<LogContextErrorAndDefault, _>(|| "ghal_bol initAndroidAudio".to_string());
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
