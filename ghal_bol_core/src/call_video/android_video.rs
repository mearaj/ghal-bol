//! Android camera capture inside this process, via the NDK Camera2 API.
//! Frames are YUV_420_888 from `AImageReader` and converted to I420 for the encoder.

#[cfg(target_os = "android")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(target_os = "android")]
use std::sync::{Mutex, OnceLock};
#[cfg(target_os = "android")]
use tokio::sync::mpsc;

#[cfg(target_os = "android")]
use super::RawVideoFrame;
#[cfg(target_os = "android")]
use super::quality::{CAP_HEIGHT, CAP_WIDTH};
#[cfg(target_os = "android")]
use super::session::VideoControls;

#[cfg(target_os = "android")]
static FRAME_TX: OnceLock<Mutex<Option<mpsc::Sender<RawVideoFrame>>>> = OnceLock::new();
#[cfg(target_os = "android")]
static CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "android")]
static FRAMES_RX: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "android")]
fn frame_tx() -> &'static Mutex<Option<mpsc::Sender<RawVideoFrame>>> {
    FRAME_TX.get_or_init(|| Mutex::new(None))
}

/// Pack Android `YUV_420_888` planes into I420 (`Y` then `U` then `V`).
pub fn yuv420888_to_i420(
    width: u32,
    height: u32,
    y: &[u8],
    y_row: usize,
    u: &[u8],
    u_row: usize,
    u_pix: usize,
    v: &[u8],
    v_row: usize,
    v_pix: usize,
) -> Option<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 || y_row < w || u_pix == 0 || v_pix == 0 {
        return None;
    }
    let y_need = y_row * (h - 1) + w;
    let uv_h = h / 2;
    let uv_w = w / 2;
    let u_need = u_row * (uv_h - 1) + (uv_w - 1) * u_pix + 1;
    let v_need = v_row * (uv_h - 1) + (uv_w - 1) * v_pix + 1;
    if y.len() < y_need || u.len() < u_need || v.len() < v_need {
        return None;
    }
    let mut out = vec![0u8; w * h + 2 * uv_w * uv_h];
    for row in 0..h {
        let src = row * y_row;
        let dst = row * w;
        out[dst..dst + w].copy_from_slice(&y[src..src + w]);
    }
    let u_off = w * h;
    let v_off = u_off + uv_w * uv_h;
    for row in 0..uv_h {
        for col in 0..uv_w {
            out[u_off + row * uv_w + col] = u[row * u_row + col * u_pix];
            out[v_off + row * uv_w + col] = v[row * v_row + col * v_pix];
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planar_yuv_packs_to_i420() {
        let y = [1u8, 2, 3, 4];
        let u = [5u8];
        let v = [6u8];
        let out = yuv420888_to_i420(2, 2, &y, 2, &u, 1, 1, &v, 1, 1).unwrap();
        assert_eq!(&out[..4], &[1, 2, 3, 4]);
        assert_eq!(out[4], 5);
        assert_eq!(out[5], 6);
    }

    #[test]
    fn semi_planar_chroma_uses_pixel_stride() {
        let y = [9u8, 8, 7, 6];
        let uv = [1u8, 2];
        let out = yuv420888_to_i420(2, 2, &y, 2, &uv, 2, 2, &uv[1..], 2, 2).unwrap();
        assert_eq!(out[4], 1);
        assert_eq!(out[5], 2);
    }
}

#[cfg(target_os = "android")]
mod ndk_camera {
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_void};
    use std::sync::{Mutex, OnceLock};

    use super::*;

    const AIMAGE_FORMAT_YUV_420_888: u32 = 35;
    const TEMPLATE_PREVIEW: u32 = 1;
    const ACAMERA_LENS_FACING: u32 = 524293;
    const ACAMERA_LENS_FACING_FRONT: u8 = 0;

    #[repr(C)]
    struct ACameraManager {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct ACameraIdList {
        num_cameras: c_int,
        camera_ids: *mut *const c_char,
    }
    #[repr(C)]
    struct ACameraMetadata {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct ACameraMetadataEntry {
        tag: u32,
        type_: u8,
        count: u32,
        data: EntryData,
    }
    #[repr(C)]
    union EntryData {
        u8_: *const u8,
        i32_: *const i32,
        f: *const f32,
        i64_: *const i64,
        d: *const f64,
        r: *const c_void,
    }
    #[repr(C)]
    struct ACameraDevice {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct ACaptureRequest {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct ANativeWindow {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct AImageReader {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct AImage {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct ACaptureSessionOutput {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct ACaptureSessionOutputContainer {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct ACameraOutputTarget {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct ACameraCaptureSession {
        _unused: [u8; 0],
    }
    #[repr(C)]
    struct DeviceCallbacks {
        context: *mut c_void,
        on_disconnected: Option<unsafe extern "C" fn(*mut c_void, *mut ACameraDevice)>,
        on_error: Option<unsafe extern "C" fn(*mut c_void, *mut ACameraDevice, c_int)>,
    }
    #[repr(C)]
    struct ImageListener {
        context: *mut c_void,
        on_image: Option<unsafe extern "C" fn(*mut c_void, *mut AImageReader)>,
    }
    #[repr(C)]
    struct SessionCallbacks {
        context: *mut c_void,
        on_closed: Option<unsafe extern "C" fn(*mut c_void, *mut ACameraCaptureSession)>,
        on_ready: Option<unsafe extern "C" fn(*mut c_void, *mut ACameraCaptureSession)>,
        on_active: Option<unsafe extern "C" fn(*mut c_void, *mut ACameraCaptureSession)>,
    }
    #[repr(C)]
    struct CaptureCallbacks {
        context: *mut c_void,
        on_started: Option<unsafe extern "C" fn(*mut c_void, *mut ACameraCaptureSession, *const ACaptureRequest, i64)>,
        on_progressed: *mut c_void,
        on_completed: *mut c_void,
        on_failed: *mut c_void,
        on_sequence_completed: *mut c_void,
        on_sequence_aborted: *mut c_void,
        on_buffer_lost: *mut c_void,
    }

    #[link(name = "mediandk")]
    unsafe extern "C" {
        fn AImageReader_new(width: i32, height: i32, format: u32, max_images: i32, reader: *mut *mut AImageReader) -> c_int;
        fn AImageReader_setImageListener(reader: *mut AImageReader, listener: *mut ImageListener) -> c_int;
        fn AImageReader_getWindow(reader: *mut AImageReader, window: *mut *mut ANativeWindow) -> c_int;
        fn AImageReader_delete(reader: *mut AImageReader);
        fn AImageReader_acquireLatestImage(reader: *mut AImageReader, image: *mut *mut AImage) -> c_int;
        fn AImage_getPlaneData(image: *const AImage, plane: c_int, data: *mut *mut u8, len: *mut c_int) -> c_int;
        fn AImage_getPlaneRowStride(image: *const AImage, plane: c_int, stride: *mut c_int) -> c_int;
        fn AImage_getPlanePixelStride(image: *const AImage, plane: c_int, stride: *mut c_int) -> c_int;
        fn AImage_delete(image: *mut AImage);
    }
    #[link(name = "nativewindow")]
    unsafe extern "C" {
        fn ANativeWindow_acquire(window: *mut ANativeWindow);
        fn ANativeWindow_release(window: *mut ANativeWindow);
    }
    #[link(name = "camera2ndk")]
    unsafe extern "C" {
        fn ACameraManager_create() -> *mut ACameraManager;
        fn ACameraManager_delete(manager: *mut ACameraManager);
        fn ACameraManager_getCameraIdList(manager: *mut ACameraManager, list: *mut *mut ACameraIdList) -> c_int;
        fn ACameraManager_deleteCameraIdList(list: *mut ACameraIdList);
        fn ACameraManager_getCameraCharacteristics(manager: *mut ACameraManager, id: *const c_char, meta: *mut *mut ACameraMetadata) -> c_int;
        fn ACameraMetadata_free(metadata: *mut ACameraMetadata);
        fn ACameraMetadata_getConstEntry(metadata: *const ACameraMetadata, tag: u32, entry: *mut ACameraMetadataEntry) -> c_int;
        fn ACameraManager_openCamera(manager: *mut ACameraManager, id: *const c_char, cb: *mut DeviceCallbacks, device: *mut *mut ACameraDevice) -> c_int;
        fn ACameraDevice_createCaptureRequest(device: *const ACameraDevice, template: u32, request: *mut *mut ACaptureRequest) -> c_int;
        fn ACameraOutputTarget_create(window: *mut ANativeWindow, output: *mut *mut ACameraOutputTarget) -> c_int;
        fn ACaptureRequest_addTarget(request: *mut ACaptureRequest, output: *const ACameraOutputTarget) -> c_int;
        fn ACaptureSessionOutput_create(window: *mut ANativeWindow, output: *mut *mut ACaptureSessionOutput) -> c_int;
        fn ACaptureSessionOutputContainer_create(container: *mut *mut ACaptureSessionOutputContainer) -> c_int;
        fn ACaptureSessionOutputContainer_add(container: *mut ACaptureSessionOutputContainer, output: *const ACaptureSessionOutput) -> c_int;
        fn ACameraDevice_createCaptureSession(device: *mut ACameraDevice, outputs: *const ACaptureSessionOutputContainer, cb: *const SessionCallbacks, session: *mut *mut ACameraCaptureSession) -> c_int;
        fn ACameraCaptureSession_setRepeatingRequest(session: *mut ACameraCaptureSession, cb: *mut CaptureCallbacks, num: c_int, requests: *mut *mut ACaptureRequest, seq: *mut c_int) -> c_int;
        fn ACameraCaptureSession_stopRepeating(session: *mut ACameraCaptureSession) -> c_int;
        fn ACameraCaptureSession_close(session: *mut ACameraCaptureSession);
        fn ACaptureSessionOutputContainer_free(container: *mut ACaptureSessionOutputContainer);
        fn ACaptureSessionOutput_free(output: *mut ACaptureSessionOutput);
        fn ACameraDevice_close(device: *mut ACameraDevice) -> c_int;
        fn ACaptureRequest_free(request: *mut ACaptureRequest);
        fn ACameraOutputTarget_free(output: *mut ACameraOutputTarget);
    }

    struct Live {
        manager: *mut ACameraManager,
        device: *mut ACameraDevice,
        request: *mut ACaptureRequest,
        reader: *mut AImageReader,
        window: *mut ANativeWindow,
        target: *mut ACameraOutputTarget,
        output: *mut ACaptureSessionOutput,
        session: *mut ACameraCaptureSession,
        _device_cb: Box<DeviceCallbacks>,
        _image_cb: Box<ImageListener>,
        _session_cb: Box<SessionCallbacks>,
        _capture_cb: Box<CaptureCallbacks>,
    }
    unsafe impl Send for Live {}

    static LIVE: OnceLock<Mutex<Option<Live>>> = OnceLock::new();

    fn live() -> &'static Mutex<Option<Live>> {
        LIVE.get_or_init(|| Mutex::new(None))
    }

    unsafe extern "C" fn on_image(_ctx: *mut c_void, reader: *mut AImageReader) {
        unsafe {
        if !CAPTURE_ACTIVE.load(Ordering::Relaxed) {
            return;
        }
        let mut image = std::ptr::null_mut();
        if AImageReader_acquireLatestImage(reader, &mut image) != 0 || image.is_null() {
            return;
        }
        let frame = read_i420(image);
        AImage_delete(image);
        let Some(frame) = frame else {
            return;
        };
        if let Ok(guard) = frame_tx().lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.try_send(frame);
                let n = FRAMES_RX.fetch_add(1, Ordering::Relaxed) + 1;
                if n == 1 {
                    crate::p2p::native_log::info("call_video", "android camera first frame");
                }
            }
        }
        }
    }

    fn read_i420(image: *mut AImage) -> Option<RawVideoFrame> {
        unsafe {
        let mut y_ptr = std::ptr::null_mut();
        let mut u_ptr = std::ptr::null_mut();
        let mut v_ptr = std::ptr::null_mut();
        let mut y_len = 0;
        let mut u_len = 0;
        let mut v_len = 0;
        let mut y_row = 0;
        let mut u_row = 0;
        let mut v_row = 0;
        let mut u_pix = 0;
        let mut v_pix = 0;
        if AImage_getPlaneData(image, 0, &mut y_ptr, &mut y_len) != 0
            || AImage_getPlaneData(image, 1, &mut u_ptr, &mut u_len) != 0
            || AImage_getPlaneData(image, 2, &mut v_ptr, &mut v_len) != 0
            || AImage_getPlaneRowStride(image, 0, &mut y_row) != 0
            || AImage_getPlaneRowStride(image, 1, &mut u_row) != 0
            || AImage_getPlaneRowStride(image, 2, &mut v_row) != 0
            || AImage_getPlanePixelStride(image, 1, &mut u_pix) != 0
            || AImage_getPlanePixelStride(image, 2, &mut v_pix) != 0
        {
            return None;
        }
        if y_ptr.is_null() || u_ptr.is_null() || v_ptr.is_null() {
            return None;
        }
        let y = std::slice::from_raw_parts(y_ptr, y_len.max(0) as usize);
        let u = std::slice::from_raw_parts(u_ptr, u_len.max(0) as usize);
        let v = std::slice::from_raw_parts(v_ptr, v_len.max(0) as usize);
        let data = yuv420888_to_i420(
            CAP_WIDTH,
            CAP_HEIGHT,
            y,
            y_row.max(0) as usize,
            u,
            u_row.max(0) as usize,
            u_pix.max(1) as usize,
            v,
            v_row.max(0) as usize,
            v_pix.max(1) as usize,
        )?;
        Some(RawVideoFrame {
            width: CAP_WIDTH,
            height: CAP_HEIGHT,
            data,
        })
        }
    }

    unsafe extern "C" fn on_disconnected(_ctx: *mut c_void, _device: *mut ACameraDevice) {}
    unsafe extern "C" fn on_error(_ctx: *mut c_void, _device: *mut ACameraDevice, _err: c_int) {}
    unsafe extern "C" fn on_session(_ctx: *mut c_void, _session: *mut ACameraCaptureSession) {}

    fn front_or_first(manager: *mut ACameraManager) -> Result<CString, String> {
        unsafe {
        let mut list = std::ptr::null_mut();
        if ACameraManager_getCameraIdList(manager, &mut list) != 0 || list.is_null() {
            return Err("no camera".into());
        }
        let n = (*list).num_cameras.max(0) as usize;
        if n == 0 || (*list).camera_ids.is_null() {
            ACameraManager_deleteCameraIdList(list);
            return Err("no camera".into());
        }
        let ids = std::slice::from_raw_parts((*list).camera_ids, n);
        let mut chosen = CStr::from_ptr(ids[0]).to_owned();
        for id in ids {
            if id.is_null() {
                continue;
            }
            let mut meta = std::ptr::null_mut();
            if ACameraManager_getCameraCharacteristics(manager, *id, &mut meta) != 0 || meta.is_null() {
                continue;
            }
            let mut entry = std::mem::zeroed::<ACameraMetadataEntry>();
            let front = ACameraMetadata_getConstEntry(meta, ACAMERA_LENS_FACING, &mut entry) == 0
                && entry.count > 0
                && !entry.data.u8_.is_null()
                && *entry.data.u8_ == ACAMERA_LENS_FACING_FRONT;
            ACameraMetadata_free(meta);
            if front {
                chosen = CStr::from_ptr(*id).to_owned();
                break;
            }
        }
        ACameraManager_deleteCameraIdList(list);
        Ok(chosen)
        }
    }

    pub fn start() -> Result<(), String> {
        stop();
        unsafe {
            let manager = ACameraManager_create();
            if manager.is_null() {
                return Err("camera manager unavailable".into());
            }
            let id = match front_or_first(manager) {
                Ok(id) => id,
                Err(e) => {
                    ACameraManager_delete(manager);
                    return Err(e);
                }
            };
            let mut device_cb = Box::new(DeviceCallbacks {
                context: std::ptr::null_mut(),
                on_disconnected: Some(on_disconnected),
                on_error: Some(on_error),
            });
            let mut device = std::ptr::null_mut();
            if ACameraManager_openCamera(manager, id.as_ptr(), device_cb.as_mut(), &mut device) != 0
                || device.is_null()
            {
                ACameraManager_delete(manager);
                return Err("could not open the camera".into());
            }
            let mut request = std::ptr::null_mut();
            if ACameraDevice_createCaptureRequest(device, TEMPLATE_PREVIEW, &mut request) != 0 {
                ACameraDevice_close(device);
                ACameraManager_delete(manager);
                return Err("could not create a capture request".into());
            }
            let mut reader = std::ptr::null_mut();
            if AImageReader_new(CAP_WIDTH as i32, CAP_HEIGHT as i32, AIMAGE_FORMAT_YUV_420_888, 4, &mut reader) != 0
                || reader.is_null()
            {
                ACaptureRequest_free(request);
                ACameraDevice_close(device);
                ACameraManager_delete(manager);
                return Err("could not create the camera reader".into());
            }
            let mut image_cb = Box::new(ImageListener {
                context: std::ptr::null_mut(),
                on_image: Some(on_image),
            });
            AImageReader_setImageListener(reader, image_cb.as_mut());
            let mut window = std::ptr::null_mut();
            AImageReader_getWindow(reader, &mut window);
            ANativeWindow_acquire(window);
            let mut target = std::ptr::null_mut();
            ACameraOutputTarget_create(window, &mut target);
            ACaptureRequest_addTarget(request, target);
            let mut output = std::ptr::null_mut();
            ACaptureSessionOutput_create(window, &mut output);
            let mut container = std::ptr::null_mut();
            ACaptureSessionOutputContainer_create(&mut container);
            ACaptureSessionOutputContainer_add(container, output);
            let session_cb = Box::new(SessionCallbacks {
                context: std::ptr::null_mut(),
                on_closed: Some(on_session),
                on_ready: Some(on_session),
                on_active: Some(on_session),
            });
            let mut session = std::ptr::null_mut();
            if ACameraDevice_createCaptureSession(device, container, session_cb.as_ref(), &mut session) != 0
                || session.is_null()
            {
                ACaptureSessionOutputContainer_free(container);
                AImageReader_delete(reader);
                ACaptureRequest_free(request);
                ACameraDevice_close(device);
                ACameraManager_delete(manager);
                return Err("could not start the camera session".into());
            }
            ACaptureSessionOutputContainer_free(container);
            let mut capture_cb = Box::new(CaptureCallbacks {
                context: std::ptr::null_mut(),
                on_started: None,
                on_progressed: std::ptr::null_mut(),
                on_completed: std::ptr::null_mut(),
                on_failed: std::ptr::null_mut(),
                on_sequence_completed: std::ptr::null_mut(),
                on_sequence_aborted: std::ptr::null_mut(),
                on_buffer_lost: std::ptr::null_mut(),
            });
            let mut req_ptr = request;
            let mut seq = 0;
            if ACameraCaptureSession_setRepeatingRequest(session, capture_cb.as_mut(), 1, &mut req_ptr, &mut seq) != 0 {
                ACameraCaptureSession_close(session);
                AImageReader_delete(reader);
                ACaptureRequest_free(request);
                ACameraDevice_close(device);
                ACameraManager_delete(manager);
                return Err("could not start camera preview".into());
            }
            if let Ok(mut g) = live().lock() {
                *g = Some(Live {
                    manager,
                    device,
                    request,
                    reader,
                    window,
                    target,
                    output,
                    session,
                    _device_cb: device_cb,
                    _image_cb: image_cb,
                    _session_cb: session_cb,
                    _capture_cb: capture_cb,
                });
            }
        }
        Ok(())
    }

    pub fn stop() {
        let Some(live) = live().lock().ok().and_then(|mut g| g.take()) else {
            return;
        };
        unsafe {
            if !live.session.is_null() {
                ACameraCaptureSession_stopRepeating(live.session);
                ACameraCaptureSession_close(live.session);
            }
            if !live.target.is_null() {
                ACameraOutputTarget_free(live.target);
            }
            if !live.output.is_null() {
                ACaptureSessionOutput_free(live.output);
            }
            if !live.window.is_null() {
                ANativeWindow_release(live.window);
            }
            if !live.reader.is_null() {
                AImageReader_delete(live.reader);
            }
            if !live.request.is_null() {
                ACaptureRequest_free(live.request);
            }
            if !live.device.is_null() {
                ACameraDevice_close(live.device);
            }
            if !live.manager.is_null() {
                ACameraManager_delete(live.manager);
            }
        }
    }
}

/// Start camera capture. Desktop uses nokhwa; Android uses the NDK camera.
#[cfg(target_os = "android")]
pub fn spawn(controls: VideoControls) -> Result<mpsc::Receiver<RawVideoFrame>, String> {
    let (tx, rx) = mpsc::channel::<RawVideoFrame>(4);
    if let Ok(mut g) = frame_tx().lock() {
        *g = Some(tx);
    }
    CAPTURE_ACTIVE.store(true, Ordering::Relaxed);
    if let Err(e) = ndk_camera::start() {
        CAPTURE_ACTIVE.store(false, Ordering::Relaxed);
        return Err(e);
    }
    tokio::spawn(async move {
        while !controls.is_stopped() {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        stop_capture();
    });
    Ok(rx)
}

#[cfg(target_os = "android")]
pub fn stop_capture() {
    CAPTURE_ACTIVE.store(false, Ordering::Relaxed);
    FRAMES_RX.store(0, Ordering::Relaxed);
    ndk_camera::stop();
    if let Ok(mut g) = frame_tx().lock() {
        g.take();
    }
}

#[cfg(not(target_os = "android"))]
pub fn stop_capture() {}
