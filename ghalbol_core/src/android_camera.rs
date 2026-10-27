//! Android NDK Camera2 capture shared by video calls (front camera) and QR scan (back camera).
//!
//! [`Camera::open`] starts a repeating preview into a `YUV_420_888` `AImageReader`; the caller's
//! image callback reads planes with the `AImage_*` functions. Dropping the [`Camera`] stops the
//! session and frees every NDK object in the same order on success and on a partial open.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

const AIMAGE_FORMAT_YUV_420_888: u32 = 35;
const TEMPLATE_PREVIEW: u32 = 1;
const ACAMERA_LENS_FACING: u32 = 524293;

/// Which lens to prefer; the first camera is used when none faces that way.
#[derive(Clone, Copy)]
pub(crate) enum Facing {
    Front,
    Back,
}

impl Facing {
    fn lens_facing(self) -> u8 {
        match self {
            Facing::Front => 0,
            Facing::Back => 1,
        }
    }
}

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
pub(crate) struct AImageReader {
    _unused: [u8; 0],
}
#[repr(C)]
pub(crate) struct AImage {
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
    on_image: Option<ImageCallback>,
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

/// `AImageReader_ImageListener::onImageAvailable`, called on a camera thread.
pub(crate) type ImageCallback = unsafe extern "C" fn(*mut c_void, *mut AImageReader);

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
    fn AImageReader_getWindow(reader: *mut AImageReader, window: *mut *mut ANativeWindow) -> c_int;
    fn AImageReader_delete(reader: *mut AImageReader);
    pub(crate) fn AImageReader_acquireLatestImage(
        reader: *mut AImageReader,
        image: *mut *mut AImage,
    ) -> c_int;
    pub(crate) fn AImage_getPlaneData(
        image: *const AImage,
        plane: c_int,
        data: *mut *mut u8,
        len: *mut c_int,
    ) -> c_int;
    pub(crate) fn AImage_getPlaneRowStride(
        image: *const AImage,
        plane: c_int,
        stride: *mut c_int,
    ) -> c_int;
    pub(crate) fn AImage_getPlanePixelStride(
        image: *const AImage,
        plane: c_int,
        stride: *mut c_int,
    ) -> c_int;
    pub(crate) fn AImage_delete(image: *mut AImage);
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

unsafe extern "C" fn on_disconnected(_ctx: *mut c_void, _device: *mut ACameraDevice) {}
unsafe extern "C" fn on_error(_ctx: *mut c_void, _device: *mut ACameraDevice, _err: c_int) {}
unsafe extern "C" fn on_session(_ctx: *mut c_void, _session: *mut ACameraCaptureSession) {}

fn pick_camera(manager: *mut ACameraManager, facing: Facing) -> Result<CString, String> {
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
            let wanted = ACameraMetadata_getConstEntry(meta, ACAMERA_LENS_FACING, &mut entry) == 0
                && entry.count > 0
                && !entry.data.u8_.is_null()
                && *entry.data.u8_ == facing.lens_facing();
            ACameraMetadata_free(meta);
            if wanted {
                chosen = CStr::from_ptr(*id).to_owned();
                break;
            }
        }
        ACameraManager_deleteCameraIdList(list);
        Ok(chosen)
    }
}

/// A running camera preview. Dropping it stops capture and frees the NDK objects.
pub(crate) struct Camera {
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

// The raw handles are only touched on open and drop; the camera runs its own threads.
unsafe impl Send for Camera {}

impl Camera {
    /// Open the `facing` camera at `width`×`height` and deliver frames to `on_image`.
    pub(crate) fn open(
        facing: Facing,
        width: i32,
        height: i32,
        on_image: ImageCallback,
    ) -> Result<Camera, String> {
        let mut cam = Camera {
            manager: std::ptr::null_mut(),
            device: std::ptr::null_mut(),
            request: std::ptr::null_mut(),
            reader: std::ptr::null_mut(),
            window: std::ptr::null_mut(),
            target: std::ptr::null_mut(),
            output: std::ptr::null_mut(),
            session: std::ptr::null_mut(),
            _device_cb: Box::new(DeviceCallbacks {
                context: std::ptr::null_mut(),
                on_disconnected: Some(on_disconnected),
                on_error: Some(on_error),
            }),
            _image_cb: Box::new(ImageListener {
                context: std::ptr::null_mut(),
                on_image: Some(on_image),
            }),
            _session_cb: Box::new(SessionCallbacks {
                context: std::ptr::null_mut(),
                on_closed: Some(on_session),
                on_ready: Some(on_session),
                on_active: Some(on_session),
            }),
            _capture_cb: Box::new(CaptureCallbacks {
                context: std::ptr::null_mut(),
                on_started: None,
                on_progressed: std::ptr::null_mut(),
                on_completed: std::ptr::null_mut(),
                on_failed: std::ptr::null_mut(),
                on_sequence_completed: std::ptr::null_mut(),
                on_sequence_aborted: std::ptr::null_mut(),
                on_buffer_lost: std::ptr::null_mut(),
            }),
        };
        // Every early return drops `cam`, which releases whatever was created so far.
        unsafe {
            cam.manager = ACameraManager_create();
            if cam.manager.is_null() {
                return Err("camera manager unavailable".into());
            }
            let id = pick_camera(cam.manager, facing)?;
            if ACameraManager_openCamera(
                cam.manager,
                id.as_ptr(),
                cam._device_cb.as_mut(),
                &mut cam.device,
            ) != 0
                || cam.device.is_null()
            {
                cam.device = std::ptr::null_mut();
                return Err("could not open the camera".into());
            }
            if ACameraDevice_createCaptureRequest(cam.device, TEMPLATE_PREVIEW, &mut cam.request)
                != 0
            {
                cam.request = std::ptr::null_mut();
                return Err("could not create a capture request".into());
            }
            if AImageReader_new(width, height, AIMAGE_FORMAT_YUV_420_888, 4, &mut cam.reader) != 0
                || cam.reader.is_null()
            {
                cam.reader = std::ptr::null_mut();
                return Err("could not create the camera reader".into());
            }
            AImageReader_setImageListener(cam.reader, cam._image_cb.as_mut());
            if AImageReader_getWindow(cam.reader, &mut cam.window) != 0 || cam.window.is_null() {
                cam.window = std::ptr::null_mut();
                return Err("could not create the camera reader".into());
            }
            ANativeWindow_acquire(cam.window);
            ACameraOutputTarget_create(cam.window, &mut cam.target);
            ACaptureRequest_addTarget(cam.request, cam.target);
            ACaptureSessionOutput_create(cam.window, &mut cam.output);
            let mut container = std::ptr::null_mut();
            ACaptureSessionOutputContainer_create(&mut container);
            ACaptureSessionOutputContainer_add(container, cam.output);
            let created = ACameraDevice_createCaptureSession(
                cam.device,
                container,
                cam._session_cb.as_ref(),
                &mut cam.session,
            ) == 0
                && !cam.session.is_null();
            ACaptureSessionOutputContainer_free(container);
            if !created {
                cam.session = std::ptr::null_mut();
                return Err("could not start the camera session".into());
            }
            let mut req_ptr = cam.request;
            let mut seq = 0;
            if ACameraCaptureSession_setRepeatingRequest(
                cam.session,
                cam._capture_cb.as_mut(),
                1,
                &mut req_ptr,
                &mut seq,
            ) != 0
            {
                return Err("could not start camera preview".into());
            }
        }
        Ok(cam)
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        unsafe {
            if !self.session.is_null() {
                ACameraCaptureSession_stopRepeating(self.session);
                ACameraCaptureSession_close(self.session);
            }
            if !self.target.is_null() {
                ACameraOutputTarget_free(self.target);
            }
            if !self.output.is_null() {
                ACaptureSessionOutput_free(self.output);
            }
            if !self.window.is_null() {
                ANativeWindow_release(self.window);
            }
            if !self.reader.is_null() {
                AImageReader_delete(self.reader);
            }
            if !self.request.is_null() {
                ACaptureRequest_free(self.request);
            }
            if !self.device.is_null() {
                ACameraDevice_close(self.device);
            }
            if !self.manager.is_null() {
                ACameraManager_delete(self.manager);
            }
        }
    }
}
