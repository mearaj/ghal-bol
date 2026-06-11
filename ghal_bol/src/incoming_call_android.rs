//! Android incoming-call notification from the `:p2p` process (UI may be killed).

#[cfg(target_os = "android")]
pub fn show(peer_public_key_hex: &str, _call_id: &str) {
    if !crate::call_media::android_p2p_context_ready() {
        return;
    }
    let name = short_display_name(peer_public_key_hex);
    let pk = peer_public_key_hex.trim().to_string();
    if pk.len() != 66 {
        return;
    }
    if let Err(e) = show_jni(&name, &pk) {
        crate::flow_log::warn("call", format!("android incoming call notify failed: {e}"));
    }
}

#[cfg(target_os = "android")]
pub fn dismiss() {
    if !crate::call_media::android_p2p_context_ready() {
        return;
    }
    let _ = dismiss_jni();
}

#[cfg(target_os = "android")]
fn short_display_name(pk: &str) -> String {
    let p = pk.trim();
    if p.len() >= 16 {
        format!("{}…", &p[..8])
    } else if p.is_empty() {
        "Contact".to_string()
    } else {
        p.to_string()
    }
}

#[cfg(target_os = "android")]
fn show_jni(display_name: &str, public_key_hex: &str) -> Result<(), String> {
    use jni::objects::JValue;
    use jni::{jni_sig, jni_str, JavaVM};

    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    let class = crate::android_jni_cache::daemon_native_class()?;
    let name = display_name.to_string();
    let pk = public_key_hex.to_string();
    vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let context =
            unsafe { jni::objects::JObject::from_raw(env, ctx.context() as jni::sys::jobject) };
        let jname = env.new_string(&name)?;
        let jpk = env.new_string(&pk)?;
        env.call_static_method(
            &*class,
            jni_str!("showIncomingCall"),
            jni_sig!((android.content.Context, java.lang.String, java.lang.String) -> void),
            &[
                JValue::Object(&context),
                JValue::Object(&jname),
                JValue::Object(&jpk),
            ],
        )?;
        Ok(())
    })
    .map_err(|e| format!("showIncomingCall jni: {e}"))
}

#[cfg(target_os = "android")]
fn dismiss_jni() -> Result<(), String> {
    use jni::objects::JValue;
    use jni::{jni_sig, jni_str, JavaVM};

    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    let class = crate::android_jni_cache::daemon_native_class()?;
    vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let context =
            unsafe { jni::objects::JObject::from_raw(env, ctx.context() as jni::sys::jobject) };
        env.call_static_method(
            &*class,
            jni_str!("dismissIncomingCall"),
            jni_sig!((android.content.Context) -> void),
            &[JValue::Object(&context)],
        )?;
        Ok(())
    })
    .map_err(|e| format!("dismissIncomingCall jni: {e}"))
}
