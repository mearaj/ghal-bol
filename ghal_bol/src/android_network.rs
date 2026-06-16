//! Android `:p2p` connectivity — probe [`ConnectivityManager`] from Rust (not Flutter/Kotlin policy).

#[cfg(target_os = "android")]
mod imp {
    use jni::objects::{JObject, JObjectArray, JValue};
    use jni::{jni_sig, jni_str, Env, JavaVM};

    pub fn wifi_transport_linked() -> bool {
        if !crate::call_media::android_p2p_context_ready() {
            return if_addrs_wifi_hint();
        }
        wifi_transport_linked_jni().unwrap_or_else(|_| if_addrs_wifi_hint())
    }

    fn if_addrs_wifi_hint() -> bool {
        let p = crate::p2p::network_transport::detect_local_network_profile();
        p.has_wifi_iface || p.has_rfc1918_on_wifi
    }

    fn with_connectivity_manager<F>(f: F) -> Result<bool, String>
    where
        F: FnOnce(&mut Env, &JObject) -> jni::errors::Result<bool>,
    {
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
            let context =
                unsafe { JObject::from_raw(env, ctx.context() as jni::sys::jobject) };
            let class_ctx = env.find_class(jni_str!("android/content/Context"))?;
            let conn_service = env
                .get_static_field(
                    &class_ctx,
                    jni_str!("CONNECTIVITY_SERVICE"),
                    jni_sig!(java.lang.String),
                )?
                .l()?;
            let cm = env
                .call_method(
                    &context,
                    jni_str!("getSystemService"),
                    jni_sig!((java.lang.String) -> java.lang.Object),
                    &[JValue::Object(&conn_service)],
                )?
                .l()?;
            f(env, &cm)
        })
        .map_err(|e| e.to_string())
    }

    fn wifi_transport_linked_jni() -> Result<bool, String> {
        with_connectivity_manager(|env, cm| {
            let class_caps = env.find_class(jni_str!("android/net/NetworkCapabilities"))?;
            let transport_wifi = env
                .get_static_field(&class_caps, jni_str!("TRANSPORT_WIFI"), jni_sig!(int))?
                .i()?;
            let networks = env
                .call_method(
                    cm,
                    jni_str!("getAllNetworks"),
                    jni_sig!( () -> [android.net.Network]),
                    &[],
                )?
                .l()?;
            let arr = JObjectArray::<JObject>::cast_local(env, networks)?;
            let len = arr.len(env)?;
            for i in 0..len {
                let net = arr.get_element(env, i)?;
                let caps = env
                    .call_method(
                        cm,
                        jni_str!("getNetworkCapabilities"),
                        jni_sig!((android.net.Network) -> android.net.NetworkCapabilities),
                        &[JValue::Object(&net)],
                    )?
                    .l()?;
                if caps.is_null() {
                    continue;
                }
                let has_wifi = env
                    .call_method(
                        &caps,
                        jni_str!("hasTransport"),
                        jni_sig!((int) -> boolean),
                        &[JValue::Int(transport_wifi)],
                    )?
                    .z()?;
                if has_wifi {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }
}

#[cfg(target_os = "android")]
pub use imp::wifi_transport_linked;

#[cfg(not(target_os = "android"))]
pub fn wifi_transport_linked() -> bool {
    false
}

/// `:p2p` Android connectivity callback — refresh Wi‑Fi hint and wake libp2p handover recovery.
pub fn on_connectivity_changed() {
    #[cfg(target_os = "android")]
    {
        let wifi = wifi_transport_linked();
        crate::p2p::chat_server::set_android_wifi_transport_available(wifi);
    }
    crate::p2p::notify_network_change();
}
