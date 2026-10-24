//! Android QR invite scan via NDK Camera2 (back camera). Preview + rqrr on the Y plane.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

const CAP_W: i32 = 640;
const CAP_H: i32 = 480;

enum Scan {
    Idle,
    Busy,
    Ready(Result<String, String>),
}

fn scan_slot() -> &'static Mutex<Scan> {
    static S: OnceLock<Mutex<Scan>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Scan::Idle))
}

fn cancel_flag() -> &'static AtomicBool {
    static C: OnceLock<AtomicBool> = OnceLock::new();
    C.get_or_init(|| AtomicBool::new(false))
}

struct Preview {
    generation: u64,
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

fn preview_slot() -> &'static Mutex<Option<Preview>> {
    static P: OnceLock<Mutex<Option<Preview>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

fn publish_preview_y(y: &[u8], width: u32, height: u32, y_row: usize) {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || y_row < w || y.len() < y_row * (h - 1) + w {
        return;
    }
    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        for col in 0..w {
            let g = y[row * y_row + col];
            let o = (row * w + col) * 4;
            rgba[o] = g;
            rgba[o + 1] = g;
            rgba[o + 2] = g;
            rgba[o + 3] = 255;
        }
    }
    if let Ok(mut slot) = preview_slot().lock() {
        let generation = slot
            .as_ref()
            .map(|p| p.generation.wrapping_add(1))
            .unwrap_or(1);
        *slot = Some(Preview {
            generation,
            w: width,
            h: height,
            rgba,
        });
    }
}

fn gray_from_y(y: &[u8], width: usize, height: usize, y_row: usize) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || y_row < width || y.len() < y_row * (height - 1) + width {
        return None;
    }
    let mut gray = vec![0u8; width * height];
    for row in 0..height {
        gray[row * width..row * width + width]
            .copy_from_slice(&y[row * y_row..row * y_row + width]);
    }
    Some(gray)
}

mod ndk {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_void};

    const AIMAGE_FORMAT_YUV_420_888: u32 = 35;
    const TEMPLATE_PREVIEW: u32 = 1;
    const ACAMERA_LENS_FACING: u32 = 524293;
    const ACAMERA_LENS_FACING_BACK: u8 = 1;

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
        on_started: Option<
            unsafe extern "C" fn(*mut c_void, *mut ACameraCaptureSession, *const ACaptureRequest, i64),
        >,
        on_progressed: *mut c_void,
        on_completed: *mut c_void,
        on_failed: *mut c_void,
        on_sequence_completed: *mut c_void,
        on_sequence_aborted: *mut c_void,
        on_buffer_lost: *mut c_void,
    }

    #[link(name = "mediandk")]
    unsafe extern "C" {
        fn AImageReader_new(
            width: i32,
            height: i32,
            format: u32,
            max_images: i32,
            reader: *mut *mut AImageReader,
        ) -> c_int;
        fn AImageReader_setImageListener(
            reader: *mut AImageReader,
            listener: *mut ImageListener,
        ) -> c_int;
        fn AImageReader_getWindow(reader: *mut AImageReader, window: *mut *mut ANativeWindow)
            -> c_int;
        fn AImageReader_delete(reader: *mut AImageReader);
        fn AImageReader_acquireLatestImage(
            reader: *mut AImageReader,
            image: *mut *mut AImage,
        ) -> c_int;
        fn AImage_getPlaneData(
            image: *const AImage,
            plane: c_int,
            data: *mut *mut u8,
            len: *mut c_int,
        ) -> c_int;
        fn AImage_getPlaneRowStride(image: *const AImage, plane: c_int, stride: *mut c_int)
            -> c_int;
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
        fn ACameraManager_getCameraIdList(
            manager: *mut ACameraManager,
            list: *mut *mut ACameraIdList,
        ) -> c_int;
        fn ACameraManager_deleteCameraIdList(list: *mut ACameraIdList);
        fn ACameraManager_getCameraCharacteristics(
            manager: *mut ACameraManager,
            id: *const c_char,
            meta: *mut *mut ACameraMetadata,
        ) -> c_int;
        fn ACameraMetadata_free(metadata: *mut ACameraMetadata);
        fn ACameraMetadata_getConstEntry(
            metadata: *const ACameraMetadata,
            tag: u32,
            entry: *mut ACameraMetadataEntry,
        ) -> c_int;
        fn ACameraManager_openCamera(
            manager: *mut ACameraManager,
            id: *const c_char,
            cb: *mut DeviceCallbacks,
            device: *mut *mut ACameraDevice,
        ) -> c_int;
        fn ACameraDevice_createCaptureRequest(
            device: *const ACameraDevice,
            template: u32,
            request: *mut *mut ACaptureRequest,
        ) -> c_int;
        fn ACameraOutputTarget_create(
            window: *mut ANativeWindow,
            output: *mut *mut ACameraOutputTarget,
        ) -> c_int;
        fn ACaptureRequest_addTarget(
            request: *mut ACaptureRequest,
            output: *const ACameraOutputTarget,
        ) -> c_int;
        fn ACaptureSessionOutput_create(
            window: *mut ANativeWindow,
            output: *mut *mut ACaptureSessionOutput,
        ) -> c_int;
        fn ACaptureSessionOutputContainer_create(
            container: *mut *mut ACaptureSessionOutputContainer,
        ) -> c_int;
        fn ACaptureSessionOutputContainer_add(
            container: *mut ACaptureSessionOutputContainer,
            output: *const ACaptureSessionOutput,
        ) -> c_int;
        fn ACameraDevice_createCaptureSession(
            device: *mut ACameraDevice,
            outputs: *const ACaptureSessionOutputContainer,
            cb: *const SessionCallbacks,
            session: *mut *mut ACameraCaptureSession,
        ) -> c_int;
        fn ACameraCaptureSession_setRepeatingRequest(
            session: *mut ACameraCaptureSession,
            cb: *mut CaptureCallbacks,
            num: c_int,
            requests: *mut *mut ACaptureRequest,
            seq: *mut c_int,
        ) -> c_int;
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

    fn live() -> &'static Mutex<Option<Live>> {
        static L: OnceLock<Mutex<Option<Live>>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(None))
    }

    static CAPTURE: AtomicBool = AtomicBool::new(false);
    static DECODE_BUSY: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn on_disconnected(_ctx: *mut c_void, _device: *mut ACameraDevice) {}
    unsafe extern "C" fn on_error(_ctx: *mut c_void, _device: *mut ACameraDevice, _err: c_int) {}
    unsafe extern "C" fn on_session(_ctx: *mut c_void, _session: *mut ACameraCaptureSession) {}

    unsafe extern "C" fn on_image(_ctx: *mut c_void, reader: *mut AImageReader) {
        unsafe {
            if !CAPTURE.load(Ordering::Relaxed) || cancel_flag().load(Ordering::Relaxed) {
                return;
            }
            let mut image = std::ptr::null_mut();
            if AImageReader_acquireLatestImage(reader, &mut image) != 0 || image.is_null() {
                return;
            }
            let mut y_ptr = std::ptr::null_mut();
            let mut y_len = 0;
            let mut y_row = 0;
            let ok = AImage_getPlaneData(image, 0, &mut y_ptr, &mut y_len) == 0
                && AImage_getPlaneRowStride(image, 0, &mut y_row) == 0
                && !y_ptr.is_null()
                && y_row > 0;
            if !ok {
                AImage_delete(image);
                return;
            }
            let y = std::slice::from_raw_parts(y_ptr, y_len.max(0) as usize);
            let row = y_row.max(0) as usize;
            publish_preview_y(y, CAP_W as u32, CAP_H as u32, row);
            if let Some(gray) = gray_from_y(y, CAP_W as usize, CAP_H as usize, row) {
                // Drop frames while a decode is in flight (never stall the camera callback).
                if !DECODE_BUSY.swap(true, Ordering::AcqRel) {
                    std::thread::Builder::new()
                        .name("ghalbol-qr-decode".into())
                        .spawn(move || {
                            if cancel_flag().load(Ordering::Relaxed) {
                                DECODE_BUSY.store(false, Ordering::Release);
                                return;
                            }
                            if let Ok(text) =
                                crate::qr_luma::decode_luma(CAP_W as usize, CAP_H as usize, &gray)
                            {
                                if crate::qr_luma::is_ghalbol_invite(&text) {
                                    crate::p2p::native_log::info(
                                        "qr",
                                        format!("android invite QR ({} chars)", text.len()),
                                    );
                                    if let Ok(mut slot) = scan_slot().lock() {
                                        *slot = Scan::Ready(Ok(text));
                                    }
                                    cancel_flag().store(true, Ordering::SeqCst);
                                    CAPTURE.store(false, Ordering::Relaxed);
                                }
                            }
                            DECODE_BUSY.store(false, Ordering::Release);
                        })
                        .ok();
                }
            }
            AImage_delete(image);
        }
    }

    fn back_or_first(manager: *mut ACameraManager) -> Result<CString, String> {
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
                if ACameraManager_getCameraCharacteristics(manager, *id, &mut meta) != 0
                    || meta.is_null()
                {
                    continue;
                }
                let mut entry = std::mem::zeroed::<ACameraMetadataEntry>();
                let back = ACameraMetadata_getConstEntry(meta, ACAMERA_LENS_FACING, &mut entry) == 0
                    && entry.count > 0
                    && !entry.data.u8_.is_null()
                    && *entry.data.u8_ == ACAMERA_LENS_FACING_BACK;
                ACameraMetadata_free(meta);
                if back {
                    chosen = CStr::from_ptr(*id).to_owned();
                    break;
                }
            }
            ACameraManager_deleteCameraIdList(list);
            Ok(chosen)
        }
    }

    pub fn start_camera() -> Result<(), String> {
        stop_camera();
        unsafe {
            let manager = ACameraManager_create();
            if manager.is_null() {
                return Err("camera manager unavailable".into());
            }
            let id = match back_or_first(manager) {
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
            if AImageReader_new(CAP_W, CAP_H, AIMAGE_FORMAT_YUV_420_888, 4, &mut reader) != 0
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
            if ACameraDevice_createCaptureSession(
                device,
                container,
                session_cb.as_ref(),
                &mut session,
            ) != 0
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
            if ACameraCaptureSession_setRepeatingRequest(
                session,
                capture_cb.as_mut(),
                1,
                &mut req_ptr,
                &mut seq,
            ) != 0
            {
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
            CAPTURE.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn stop_camera() {
        CAPTURE.store(false, Ordering::Relaxed);
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

pub fn start_qr_scan() -> Result<(), String> {
    cancel_flag().store(true, Ordering::SeqCst);
    // Brief wait if a prior scan is tearing down.
    for _ in 0..40 {
        if let Ok(slot) = scan_slot().lock() {
            if !matches!(*slot, Scan::Busy) {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    cancel_flag().store(false, Ordering::SeqCst);
    {
        let mut slot = scan_slot().lock().map_err(|e| e.to_string())?;
        if matches!(*slot, Scan::Busy) {
            return Err("Camera is still stopping — try again".into());
        }
        *slot = Scan::Busy;
    }
    if let Ok(mut p) = preview_slot().lock() {
        *p = None;
    }
    crate::p2p::native_log::info("qr", "android scan start — opening back camera");
    if let Err(e) = ndk::start_camera() {
        if let Ok(mut slot) = scan_slot().lock() {
            *slot = Scan::Idle;
        }
        return Err(e);
    }
    Ok(())
}

pub fn take_qr_scan() -> Option<Result<String, String>> {
    let mut slot = scan_slot().lock().ok()?;
    match std::mem::replace(&mut *slot, Scan::Idle) {
        Scan::Ready(result) => {
            ndk::stop_camera();
            Some(result)
        }
        other => {
            *slot = other;
            None
        }
    }
}

pub fn take_qr_preview_rgba() -> Option<(u32, u32, Vec<u8>)> {
    let mut slot = preview_slot().lock().ok()?;
    let preview = slot.take()?;
    Some((preview.w, preview.h, preview.rgba))
}

pub fn stop_qr_scan() {
    cancel_flag().store(true, Ordering::SeqCst);
    ndk::stop_camera();
    if let Ok(mut slot) = scan_slot().lock() {
        if matches!(*slot, Scan::Busy) {
            *slot = Scan::Ready(Err("Scan stopped".into()));
        }
    }
    if let Ok(mut p) = preview_slot().lock() {
        *p = None;
    }
}
