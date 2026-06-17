//! Cached JNI global refs for `:p2p` — `find_class` from tokio/native worker threads
//! fails on Android; cache during `initAndroidAudio` on the JVM main attach path.

#[cfg(target_os = "android")]
mod imp {
    use std::sync::OnceLock;

    use jni::Env;
    use jni::objects::{Global, JClass};

    static DAEMON_NATIVE_CLASS: OnceLock<Global<JClass<'static>>> = OnceLock::new();

    pub fn cache_daemon_native_class<'local>(
        env: &mut Env<'local>,
        class: &JClass<'local>,
    ) -> jni::errors::Result<()> {
        if DAEMON_NATIVE_CLASS.get().is_some() {
            return Ok(());
        }
        let global = env.new_global_ref(class)?;
        let _ = DAEMON_NATIVE_CLASS.set(global);
        Ok(())
    }

    pub fn daemon_native_class() -> Result<&'static Global<JClass<'static>>, String> {
        DAEMON_NATIVE_CLASS.get().ok_or_else(|| {
            "P2pDaemonNative JNI class not cached — :p2p initAndroidAudio must run first"
                .to_string()
        })
    }
}

#[cfg(target_os = "android")]
pub use imp::{cache_daemon_native_class, daemon_native_class};
