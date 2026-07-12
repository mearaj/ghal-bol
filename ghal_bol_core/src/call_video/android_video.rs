//! Android `:p2p` camera → I420 frames for the native video engine.
//!
//! Camera2 runs in Kotlin (`AndroidVideoCapture.kt`) because the NDK has no stable
//! camera API. Kotlin pushes I420 bytes via JNI; this module forwards them into the
//! same `mpsc` channel `spawn_camera_capture` uses on desktop (nokhwa).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc;

use super::RawVideoFrame;
use super::session::VideoControls;

static FRAME_TX: OnceLock<Mutex<Option<mpsc::Sender<RawVideoFrame>>>> = OnceLock::new();
static CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
static JNI_FRAMES_RX: AtomicU64 = AtomicU64::new(0);

fn frame_tx() -> &'static Mutex<Option<mpsc::Sender<RawVideoFrame>>> {
    FRAME_TX.get_or_init(|| Mutex::new(None))
}

/// JNI: Kotlin pushes one I420 frame from Camera2.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_com_ghalbol_P2pDaemonNative_pushCameraFrame<'local>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: jni::objects::JClass<'local>,
    data: jni::objects::JByteArray<'local>,
    width: jni::sys::jint,
    height: jni::sys::jint,
) {
    if !CAPTURE_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let w = width.max(0) as u32;
    let h = height.max(0) as u32;
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
        return;
    }
    let expected = (w as usize) * (h as usize) + 2 * ((w as usize / 2) * (h as usize / 2));
    let _ = unowned_env.with_env(|env| -> jni::errors::Result<()> {
        let bytes = env.convert_byte_array(&data)?;
        if bytes.len() < expected {
            return Ok(());
        }
        if let Ok(guard) = frame_tx().lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.try_send(RawVideoFrame {
                    width: w,
                    height: h,
                    data: bytes,
                });
                let n = JNI_FRAMES_RX.fetch_add(1, Ordering::Relaxed) + 1;
                if n == 1 {
                    crate::p2p::native_log::info(
                        "call_video",
                        format!("jni first camera frame {w}x{h}"),
                    );
                }
            }
        }
        Ok(())
    });
}

#[cfg(target_os = "android")]
fn start_camera_jni() -> Result<(), String> {
    use jni::objects::JValue;
    use jni::{JavaVM, jni_sig, jni_str};
    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    let class = crate::android_jni_cache::daemon_native_class()?;
    vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let context =
            unsafe { jni::objects::JObject::from_raw(env, ctx.context() as jni::sys::jobject) };
        env.call_static_method(
            &*class,
            jni_str!("startCameraCapture"),
            jni_sig!((android.content.Context) -> void),
            &[JValue::Object(&context)],
        )?;
        Ok(())
    })
    .map_err(|e| format!("startCameraCapture jni: {e}"))
}

#[cfg(target_os = "android")]
fn stop_camera_jni() {
    use jni::{JavaVM, jni_sig, jni_str};
    CAPTURE_ACTIVE.store(false, Ordering::Relaxed);
    JNI_FRAMES_RX.store(0, Ordering::Relaxed);
    if let Ok(mut g) = frame_tx().lock() {
        g.take();
    }
    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    if let Ok(class) = crate::android_jni_cache::daemon_native_class() {
        let _ = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            env.call_static_method(
                &*class,
                jni_str!("stopCameraCapture"),
                jni_sig!(() -> void),
                &[],
            )?;
            Ok(())
        });
    }
}

/// Start receiving camera frames from Kotlin Camera2 into an async channel.
#[cfg(target_os = "android")]
pub fn spawn(controls: VideoControls) -> Result<mpsc::Receiver<RawVideoFrame>, String> {
    if !crate::call_media::android_p2p_context_ready() {
        return Err("android context not initialized (initAndroidAudio)".to_string());
    }
    // Kotlin `start()` stops any prior capture on the camera thread before opening.
    let (tx, rx) = mpsc::channel::<RawVideoFrame>(4);
    if let Ok(mut g) = frame_tx().lock() {
        *g = Some(tx);
    }
    CAPTURE_ACTIVE.store(true, Ordering::Relaxed);
    start_camera_jni()?;
    // Stop camera when the video session ends.
    tokio::spawn(async move {
        while !controls.is_stopped() {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        stop_capture();
    });
    Ok(rx)
}

/// Tear down Camera2 capture (call on session stop).
#[cfg(target_os = "android")]
pub fn stop_capture() {
    stop_camera_jni();
}

#[cfg(not(target_os = "android"))]
pub fn stop_capture() {}
