//! Android `:p2p` output routing — `AudioManager.setSpeakerphoneOn` only.
//!
//! `MODE_IN_COMMUNICATION` is set **once** before cpal/Oboe opens streams ([`ensure_voice_audio_mode`]).
//! Never call `setMode` from speaker toggle — that tears down an already-open Oboe stream.

#[cfg(target_os = "android")]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};

    use jni::objects::{JObject, JValue};
    use jni::{Env, JavaVM, jni_sig, jni_str};

    use super::super::audio_device::is_android_audio_ready;

    static VOICE_AUDIO_MODE_SET: AtomicBool = AtomicBool::new(false);

    fn with_audio_manager<F>(f: F) -> Result<(), String>
    where
        F: FnOnce(&mut Env, &JObject) -> jni::errors::Result<()>,
    {
        if !is_android_audio_ready() {
            return Err("android audio not initialized (initAndroidAudio)".into());
        }
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let context = unsafe { JObject::from_raw(env, ctx.context() as jni::sys::jobject) };
            let class_ctx = env.find_class(jni_str!("android/content/Context"))?;
            let audio_service = env
                .get_static_field(
                    &class_ctx,
                    jni_str!("AUDIO_SERVICE"),
                    jni_sig!(java.lang.String),
                )?
                .l()?;
            let am = env
                .call_method(
                    &context,
                    jni_str!("getSystemService"),
                    jni_sig!((java.lang.String) -> java.lang.Object),
                    &[JValue::Object(&audio_service)],
                )?
                .l()?;
            f(env, &am)
        })
        .map_err(|e| e.to_string())
    }

    /// Call once per call **before** cpal opens capture/playout (see `start_call_media`).
    pub fn ensure_voice_audio_mode() -> Result<(), String> {
        if VOICE_AUDIO_MODE_SET.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        with_audio_manager(|env, am| {
            let class_am = env.find_class(jni_str!("android/media/AudioManager"))?;
            let mode_in_comm = env
                .get_static_field(&class_am, jni_str!("MODE_IN_COMMUNICATION"), jni_sig!(int))?
                .i()?;
            env.call_method(
                am,
                jni_str!("setMode"),
                jni_sig!((int) -> void),
                &[JValue::Int(mode_in_comm)],
            )?;
            Ok(())
        })
    }

    pub fn reset_voice_audio_mode_flag() {
        VOICE_AUDIO_MODE_SET.store(false, Ordering::SeqCst);
    }

    pub fn set_speakerphone(on: bool) -> Result<(), String> {
        with_audio_manager(|env, am| {
            env.call_method(
                am,
                jni_str!("setSpeakerphoneOn"),
                jni_sig!((boolean) -> void),
                &[JValue::Bool(on)],
            )?;
            Ok(())
        })
    }
}

#[cfg(target_os = "android")]
pub use imp::{ensure_voice_audio_mode, reset_voice_audio_mode_flag, set_speakerphone};

#[cfg(not(target_os = "android"))]
pub fn set_speakerphone(_on: bool) -> Result<(), String> {
    Err("native speaker route only on Android".into())
}
