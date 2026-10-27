//! Android `:p2p` connectivity — probe [`ConnectivityManager`] from Rust (not Makepad/Kotlin policy).

#[cfg(target_os = "android")]
mod imp {
    use jni::objects::{JObject, JObjectArray, JValue};
    use jni::{Env, JavaVM, jni_sig, jni_str};

    use crate::p2p::network_transport::{
        OsDefaultTransport, OsNetworkSnapshot, detect_local_network_profile,
    };

    // android.net.NetworkCapabilities
    const NET_CAPABILITY_INTERNET: i32 = 12;
    const NET_CAPABILITY_VALIDATED: i32 = 16;
    const TRANSPORT_CELLULAR: i32 = 0;
    const TRANSPORT_WIFI: i32 = 1;
    const TRANSPORT_ETHERNET: i32 = 3;

    pub fn probe_connectivity_truth() -> OsNetworkSnapshot {
        if !crate::call_media::android_p2p_context_ready() {
            return if_addrs_fallback();
        }
        probe_connectivity_truth_jni().unwrap_or_else(|_| if_addrs_fallback())
    }

    fn if_addrs_fallback() -> OsNetworkSnapshot {
        let p = detect_local_network_profile();
        let has_active_lan = p.has_rfc1918_on_wifi
            || (p.has_rfc1918_ipv4
                && (p.has_wifi_iface
                    || p.has_tether_iface
                    || p.has_usb_iface
                    || (!p.has_cellular_iface && !p.has_cgnat_ipv4)));
        OsNetworkSnapshot {
            default_transport: if has_active_lan {
                OsDefaultTransport::Wifi
            } else if p.has_cellular_iface || p.has_cgnat_ipv4 {
                OsDefaultTransport::Cellular
            } else {
                OsDefaultTransport::None
            },
            internet_validated: false,
            has_internet: p.has_public_ipv4 || p.has_global_ipv6 || p.has_rfc1918_ipv4,
            wifi_link_up: p.has_wifi_iface || p.has_rfc1918_on_wifi,
            default_route_iface: None,
        }
    }

    /// Wi-Fi `MulticastLock` held while the node runs (see [`hold_multicast_lock`]).
    static MULTICAST_LOCK: std::sync::Mutex<Option<jni::objects::Global<JObject<'static>>>> =
        std::sync::Mutex::new(None);

    /// Android drops inbound multicast unless an app holds a Wi-Fi `MulticastLock`; without it
    /// mDNS never sees LAN peers and calls fall back to the coord bridge. Needs
    /// `CHANGE_WIFI_MULTICAST_STATE` in the manifest.
    pub fn hold_multicast_lock(hold: bool) {
        if !crate::call_media::android_p2p_context_ready() {
            return;
        }
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
        let result = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let mut slot = MULTICAST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let result = if hold {
                if slot.is_some() {
                    return Ok(());
                }
                let activity =
                    unsafe { JObject::from_raw(env, ctx.context() as jni::sys::jobject) };
                let context = env.new_local_ref(&activity)?;
                // `activity` is Makepad's global ref. Dropping it would delete it.
                std::mem::forget(activity);
                acquire_multicast_lock(env, &context).map(|lock| *slot = Some(lock))
            } else if let Some(lock) = slot.take() {
                env.call_method(&lock, jni_str!("release"), jni_sig!("()V"), &[])
                    .map(|_| ())
            } else {
                Ok(())
            };
            if env.exception_check() {
                env.exception_clear();
            }
            result
        });
        if let Err(e) = result {
            crate::p2p::native_log::warn("connect", format!("wifi multicast lock: {e}"));
        }
    }

    fn acquire_multicast_lock<'a>(
        env: &mut Env<'a>,
        context: &JObject<'a>,
    ) -> jni::errors::Result<jni::objects::Global<JObject<'static>>> {
        // Application context: an Activity-scoped WifiManager leaks on older Android.
        let app = env
            .call_method(
                context,
                jni_str!("getApplicationContext"),
                jni_sig!("()Landroid/content/Context;"),
                &[],
            )?
            .l()?;
        let class_ctx = env.find_class(jni_str!("android/content/Context"))?;
        let wifi_service = env
            .get_static_field(
                &class_ctx,
                jni_str!("WIFI_SERVICE"),
                jni_sig!(java.lang.String),
            )?
            .l()?;
        let wifi = env
            .call_method(
                &app,
                jni_str!("getSystemService"),
                jni_sig!((java.lang.String) -> java.lang.Object),
                &[JValue::Object(&wifi_service)],
            )?
            .l()?;
        if wifi.is_null() {
            return Err(jni::errors::Error::NullPtr("no WifiManager"));
        }
        let tag = env.new_string("ghalbol-mdns")?;
        let lock = env
            .call_method(
                &wifi,
                jni_str!("createMulticastLock"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/wifi/WifiManager$MulticastLock;"),
                &[JValue::Object(&tag)],
            )?
            .l()?;
        env.call_method(
            &lock,
            jni_str!("setReferenceCounted"),
            jni_sig!("(Z)V"),
            &[JValue::Bool(false)],
        )?;
        env.call_method(&lock, jni_str!("acquire"), jni_sig!("()V"), &[])?;
        env.new_global_ref(&lock)
    }

    fn with_connectivity_manager<F>(f: F) -> Result<OsNetworkSnapshot, String>
    where
        F: FnOnce(&mut Env, &JObject) -> jni::errors::Result<OsNetworkSnapshot>,
    {
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> jni::errors::Result<OsNetworkSnapshot> {
            let context = unsafe { JObject::from_raw(env, ctx.context() as jni::sys::jobject) };
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

    fn probe_connectivity_truth_jni() -> Result<OsNetworkSnapshot, String> {
        with_connectivity_manager(|env, cm| {
            let mut snap = OsNetworkSnapshot::default();
            snap.wifi_link_up = any_network_has_transport(env, cm, TRANSPORT_WIFI)?;

            let active = env
                .call_method(
                    cm,
                    jni_str!("getActiveNetwork"),
                    jni_sig!( () -> android.net.Network),
                    &[],
                )?
                .l()?;
            if active.is_null() {
                return Ok(snap);
            }
            let caps = env
                .call_method(
                    cm,
                    jni_str!("getNetworkCapabilities"),
                    jni_sig!((android.net.Network) -> android.net.NetworkCapabilities),
                    &[JValue::Object(&active)],
                )?
                .l()?;
            if caps.is_null() {
                return Ok(snap);
            }

            let has_wifi = env
                .call_method(
                    &caps,
                    jni_str!("hasTransport"),
                    jni_sig!((int) -> boolean),
                    &[JValue::Int(TRANSPORT_WIFI)],
                )?
                .z()?;
            let has_cell = env
                .call_method(
                    &caps,
                    jni_str!("hasTransport"),
                    jni_sig!((int) -> boolean),
                    &[JValue::Int(TRANSPORT_CELLULAR)],
                )?
                .z()?;
            let has_eth = env
                .call_method(
                    &caps,
                    jni_str!("hasTransport"),
                    jni_sig!((int) -> boolean),
                    &[JValue::Int(TRANSPORT_ETHERNET)],
                )?
                .z()?;

            snap.default_transport = if has_wifi {
                OsDefaultTransport::Wifi
            } else if has_eth {
                OsDefaultTransport::Ethernet
            } else if has_cell {
                OsDefaultTransport::Cellular
            } else {
                OsDefaultTransport::None
            };
            snap.has_internet = env
                .call_method(
                    &caps,
                    jni_str!("hasCapability"),
                    jni_sig!((int) -> boolean),
                    &[JValue::Int(NET_CAPABILITY_INTERNET)],
                )?
                .z()?;
            snap.internet_validated = env
                .call_method(
                    &caps,
                    jni_str!("hasCapability"),
                    jni_sig!((int) -> boolean),
                    &[JValue::Int(NET_CAPABILITY_VALIDATED)],
                )?
                .z()?;
            Ok(snap)
        })
    }

    fn any_network_has_transport(
        env: &mut Env,
        cm: &JObject,
        transport: i32,
    ) -> jni::errors::Result<bool> {
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
            let has = env
                .call_method(
                    &caps,
                    jni_str!("hasTransport"),
                    jni_sig!((int) -> boolean),
                    &[JValue::Int(transport)],
                )?
                .z()?;
            if has {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(target_os = "android")]
pub use imp::{hold_multicast_lock, probe_connectivity_truth};

#[cfg(not(target_os = "android"))]
pub fn probe_connectivity_truth() -> crate::p2p::network_transport::OsNetworkSnapshot {
    crate::p2p::network_transport::OsNetworkSnapshot::default()
}
