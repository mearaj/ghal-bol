//! Android `:p2p` connectivity — probe [`ConnectivityManager`] from Rust (not Flutter/Kotlin policy).

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
                .call_method(cm, jni_str!("getActiveNetwork"), jni_sig!( () -> android.net.Network), &[])?
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

    fn any_network_has_transport(env: &mut Env, cm: &JObject, transport: i32) -> jni::errors::Result<bool> {
        let networks = env
            .call_method(cm, jni_str!("getAllNetworks"), jni_sig!( () -> [android.net.Network]), &[])?
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
pub use imp::probe_connectivity_truth;

#[cfg(not(target_os = "android"))]
pub fn probe_connectivity_truth() -> crate::p2p::network_transport::OsNetworkSnapshot {
    crate::p2p::network_transport::OsNetworkSnapshot::default()
}

/// `:p2p` Android connectivity callback — refresh OS truth and wake libp2p handover recovery.
pub fn on_connectivity_changed() {
    crate::p2p::network_transport::refresh_os_network_truth();
    crate::p2p::notify_network_change();
}
