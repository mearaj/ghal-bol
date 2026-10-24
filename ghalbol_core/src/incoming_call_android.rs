//! Android notifications posted from this process. No extra Java class.

#[cfg(target_os = "android")]
mod imp {
    use jni::objects::{JObject, JValue};
    use jni::{JavaVM, jni_sig, jni_str};

    const CHANNEL: &str = "ghalbol";
    pub const INCOMING_ID: i32 = 71001;
    pub const UNLOCK_ID: i32 = 71002;

    fn with_context<F>(f: F) -> Result<(), String>
    where
        F: for<'local> FnOnce(&mut jni::Env<'local>, &JObject<'local>) -> jni::errors::Result<()>,
    {
        if !crate::call_media::android_p2p_context_ready() {
            return Err("android context is not installed".into());
        }
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            let activity = unsafe { JObject::from_raw(env, ctx.context() as jni::sys::jobject) };
            let context = env.new_local_ref(&activity)?;
            std::mem::forget(activity);
            f(env, &context)
        })
        .map_err(|e| e.to_string())
    }

    fn notification_manager<'a>(
        env: &mut jni::Env<'a>,
        context: &JObject<'a>,
    ) -> jni::errors::Result<JObject<'a>> {
        let class_ctx = env.find_class(jni_str!("android/content/Context"))?;
        let service = env
            .get_static_field(
                &class_ctx,
                jni_str!("NOTIFICATION_SERVICE"),
                jni_sig!(java.lang.String),
            )?
            .l()?;
        env.call_method(
            context,
            jni_str!("getSystemService"),
            jni_sig!((java.lang.String) -> java.lang.Object),
            &[JValue::Object(&service)],
        )?
        .l()
    }

    fn ensure_channel(env: &mut jni::Env, context: &JObject, nm: &JObject) -> jni::errors::Result<()> {
        let version = env.find_class(jni_str!("android/os/Build$VERSION"))?;
        let sdk = env.get_static_field(&version, jni_str!("SDK_INT"), jni_sig!(int))?.i()?;
        if sdk < 26 {
            return Ok(());
        }
        let channel_class = env.find_class(jni_str!("android/app/NotificationChannel"))?;
        let id = env.new_string(CHANNEL)?;
        let name = env.new_string("Ghal Bol")?;
        let channel = env.new_object(
            &channel_class,
            jni_sig!("(Ljava/lang/String;Ljava/lang/CharSequence;I)V"),
            &[
                JValue::Object(&id),
                JValue::Object(&name),
                JValue::Int(4),
            ],
        )?;
        env.call_method(
            nm,
            jni_str!("createNotificationChannel"),
            jni_sig!("(Landroid/app/NotificationChannel;)V"),
            &[JValue::Object(&channel)],
        )?;
        let _ = context;
        Ok(())
    }

    pub fn post(id: i32, title: &str, body: &str) -> Result<(), String> {
        let title = title.to_string();
        let body = body.to_string();
        with_context(|env, context| {
            let nm = notification_manager(env, context)?;
            ensure_channel(env, context, &nm)?;
            let intent_class = env.find_class(jni_str!("android/content/Intent"))?;
            let activity_class = env.get_object_class(context)?;
            let intent = env.new_object(
                &intent_class,
                jni_sig!("(Landroid/content/Context;Ljava/lang/Class;)V"),
                &[JValue::Object(context), JValue::Object(&activity_class)],
            )?;
            let flags = 0x20000000 | 0x04000000;
            env.call_method(
                &intent,
                jni_str!("addFlags"),
                jni_sig!("(I)Landroid/content/Intent;"),
                &[JValue::Int(flags)],
            )?;
            let pi_class = env.find_class(jni_str!("android/app/PendingIntent"))?;
            let pending = env.call_static_method(
                &pi_class,
                jni_str!("getActivity"),
                jni_sig!("(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;"),
                &[
                    JValue::Object(context),
                    JValue::Int(id),
                    JValue::Object(&intent),
                    JValue::Int(0x08000000 | 0x04000000),
                ],
            )?.l()?;
            let builder_class = env.find_class(jni_str!("android/app/Notification$Builder"))?;
            let channel = env.new_string(CHANNEL)?;
            let builder = env.new_object(
                &builder_class,
                jni_sig!("(Landroid/content/Context;Ljava/lang/String;)V"),
                &[JValue::Object(context), JValue::Object(&channel)],
            )?;
            let jtitle = env.new_string(&title)?;
            let jbody = env.new_string(&body)?;
            env.call_method(
                &builder,
                jni_str!("setContentTitle"),
                jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&jtitle)],
            )?;
            env.call_method(
                &builder,
                jni_str!("setContentText"),
                jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&jbody)],
            )?;
            let drawable = env.find_class(jni_str!("android/R$drawable"))?;
            let icon = env
                .get_static_field(&drawable, jni_str!("stat_sys_warning"), jni_sig!(int))?
                .i()?;
            env.call_method(
                &builder,
                jni_str!("setSmallIcon"),
                jni_sig!("(I)Landroid/app/Notification$Builder;"),
                &[JValue::Int(icon)],
            )?;
            env.call_method(
                &builder,
                jni_str!("setContentIntent"),
                jni_sig!("(Landroid/app/PendingIntent;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&pending)],
            )?;
            env.call_method(
                &builder,
                jni_str!("setAutoCancel"),
                jni_sig!("(Z)Landroid/app/Notification$Builder;"),
                &[JValue::Bool(true)],
            )?;
            let notification = env
                .call_method(
                    &builder,
                    jni_str!("build"),
                    jni_sig!("()Landroid/app/Notification;"),
                    &[],
                )?
                .l()?;
            env.call_method(
                &nm,
                jni_str!("notify"),
                jni_sig!("(ILandroid/app/Notification;)V"),
                &[JValue::Int(id), JValue::Object(&notification)],
            )?;
            Ok(())
        })
    }

    pub fn cancel(id: i32) -> Result<(), String> {
        with_context(|env, context| {
            let nm = notification_manager(env, context)?;
            env.call_method(
                &nm,
                jni_str!("cancel"),
                jni_sig!((int) -> void),
                &[JValue::Int(id)],
            )?;
            Ok(())
        })
    }

    pub fn view_url() -> Option<String> {
        let mut url = String::new();
        let ok = with_context(|env, context| {
            let intent = env
                .call_method(
                    context,
                    jni_str!("getIntent"),
                    jni_sig!("()Landroid/content/Intent;"),
                    &[],
                )?
                .l()?;
            if intent.is_null() {
                return Ok(());
            }
            let data = env
                .call_method(
                    &intent,
                    jni_str!("getDataString"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            if data.is_null() {
                return Ok(());
            }
            let text = unsafe { jni::objects::JString::from_raw(env, data.as_raw()) };
            url = text.try_to_string(env)?;
            std::mem::forget(data);
            Ok(())
        });
        if ok.is_err() || url.is_empty() {
            return None;
        }
        Some(url)
    }

    pub fn view_file(path: &str, mime: &str) -> Result<(), String> {
        let path = path.to_string();
        let mime = mime.to_string();
        with_context(|env, context| {
            let uri_class = env.find_class(jni_str!("android/net/Uri"))?;
            let file_uri = format!("file://{path}");
            let juri = env.new_string(&file_uri)?;
            let uri = env
                .call_static_method(
                    &uri_class,
                    jni_str!("parse"),
                    jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                    &[JValue::Object(&juri)],
                )?
                .l()?;
            let intent_class = env.find_class(jni_str!("android/content/Intent"))?;
            let action = env
                .get_static_field(
                    &intent_class,
                    jni_str!("ACTION_VIEW"),
                    jni_sig!("Ljava/lang/String;"),
                )?
                .l()?;
            let intent = env.new_object(
                &intent_class,
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&action)],
            )?;
            let jmime = env.new_string(&mime)?;
            env.call_method(
                &intent,
                jni_str!("setDataAndType"),
                jni_sig!("(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;"),
                &[JValue::Object(&uri), JValue::Object(&jmime)],
            )?;
            env.call_method(
                &intent,
                jni_str!("addFlags"),
                jni_sig!("(I)Landroid/content/Intent;"),
                &[JValue::Int(0x10000000)],
            )?;
            env.call_method(
                context,
                jni_str!("startActivity"),
                jni_sig!("(Landroid/content/Intent;)V"),
                &[JValue::Object(&intent)],
            )?;
            Ok(())
        })
    }

    pub fn files_dir() -> Result<String, String> {
        let mut path = String::new();
        with_context(|env, context| {
            let file = env
                .call_method(
                    context,
                    jni_str!("getFilesDir"),
                    jni_sig!(() -> java.io.File),
                    &[],
                )?
                .l()?;
            let abs = env
                .call_method(
                    &file,
                    jni_str!("getAbsolutePath"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            let text = unsafe { jni::objects::JString::from_raw(env, abs.as_raw()) };
            path = text.try_to_string(env)?;
            std::mem::forget(abs);
            Ok(())
        })?;
        if path.is_empty() {
            return Err("android files directory is empty".into());
        }
        Ok(path)
    }

    /// Write keystore JSON to app files + MediaStore Downloads when possible.
    pub fn save_keystore_backup(json: &str) -> Result<String, String> {
        let json = json.to_string();
        let files = files_dir()?;
        let backup_dir = std::path::PathBuf::from(&files).join("backups");
        std::fs::create_dir_all(&backup_dir).map_err(|e| format!("backup dir: {e}"))?;
        let local = backup_dir.join("ghalbol-keystore.json");
        std::fs::write(&local, json.as_bytes()).map_err(|e| format!("write backup: {e}"))?;

        let mut downloads_ok = false;
        let media_err = with_context(|env, context| {
            let resolver = env
                .call_method(
                    context,
                    jni_str!("getContentResolver"),
                    jni_sig!(() -> android.content.ContentResolver),
                    &[],
                )?
                .l()?;
            let values_class = env.find_class(jni_str!("android/content/ContentValues"))?;
            let values = env.new_object(&values_class, jni_sig!("()V"), &[])?;
            let put_str = |env: &mut jni::Env, values: &JObject, key: &str, val: &str| {
                let jkey = env.new_string(key)?;
                let jval = env.new_string(val)?;
                env.call_method(
                    values,
                    jni_str!("put"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
                    &[JValue::Object(&jkey), JValue::Object(&jval)],
                )?;
                Ok::<(), jni::errors::Error>(())
            };
            let put_int = |env: &mut jni::Env, values: &JObject, key: &str, val: i32| {
                let jkey = env.new_string(key)?;
                let integer_class = env.find_class(jni_str!("java/lang/Integer"))?;
                let boxed = env.new_object(
                    &integer_class,
                    jni_sig!("(I)V"),
                    &[JValue::Int(val)],
                )?;
                env.call_method(
                    values,
                    jni_str!("put"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/Integer;)V"),
                    &[JValue::Object(&jkey), JValue::Object(&boxed)],
                )?;
                Ok::<(), jni::errors::Error>(())
            };
            put_str(env, &values, "_display_name", "ghalbol-keystore.json")?;
            put_str(env, &values, "mime_type", "application/json")?;
            put_str(env, &values, "relative_path", "Download/")?;
            put_int(env, &values, "is_pending", 1)?;

            let downloads = env.find_class(jni_str!("android/provider/MediaStore$Downloads"))?;
            let collection = env
                .get_static_field(
                    &downloads,
                    jni_str!("EXTERNAL_CONTENT_URI"),
                    jni_sig!(android.net.Uri),
                )?
                .l()?;
            let uri = env
                .call_method(
                    &resolver,
                    jni_str!("insert"),
                    jni_sig!("(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;"),
                    &[JValue::Object(&collection), JValue::Object(&values)],
                )?
                .l()?;
            if uri.is_null() {
                return Err(jni::errors::Error::NullPtr("MediaStore insert returned null"));
            }
            let stream = env
                .call_method(
                    &resolver,
                    jni_str!("openOutputStream"),
                    jni_sig!("(Landroid/net/Uri;)Ljava/io/OutputStream;"),
                    &[JValue::Object(&uri)],
                )?
                .l()?;
            if stream.is_null() {
                return Err(jni::errors::Error::NullPtr("openOutputStream returned null"));
            }
            let bytes = env.byte_array_from_slice(json.as_bytes())?;
            env.call_method(
                &stream,
                jni_str!("write"),
                jni_sig!("([B)V"),
                &[JValue::Object(bytes.as_ref())],
            )?;
            env.call_method(&stream, jni_str!("flush"), jni_sig!("()V"), &[])?;
            env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[])?;

            let done = env.new_object(&values_class, jni_sig!("()V"), &[])?;
            put_int(env, &done, "is_pending", 0)?;
            let null_obj = JObject::null();
            env.call_method(
                &resolver,
                jni_str!("update"),
                jni_sig!("(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I"),
                &[
                    JValue::Object(&uri),
                    JValue::Object(&done),
                    JValue::Object(&null_obj),
                    JValue::Object(&null_obj),
                ],
            )?;
            downloads_ok = true;
            Ok(())
        });

        if downloads_ok {
            Ok(
                "Saved to Downloads as ghalbol-keystore.json (also kept under app files). Keep this file and your app password. Older backups still use the password they were exported with."
                    .into(),
            )
        } else {
            let _ = media_err;
            Ok(format!(
                "Saved {}. Keep this file and your app password. Older backups still use the password they were exported with.",
                local.display()
            ))
        }
    }

    /// Write `bytes` into MediaStore Downloads as `display_name` with `mime`.
    pub fn save_bytes_to_downloads(
        bytes: &[u8],
        display_name: &str,
        mime: &str,
    ) -> Result<String, String> {
        let display_name = display_name.trim();
        if display_name.is_empty() {
            return Err("missing file name".into());
        }
        let mime = if mime.trim().is_empty() {
            "application/octet-stream"
        } else {
            mime
        };
        let bytes = bytes.to_vec();
        let mut downloads_ok = false;
        let media_err = with_context(|env, context| {
            let resolver = env
                .call_method(
                    context,
                    jni_str!("getContentResolver"),
                    jni_sig!(() -> android.content.ContentResolver),
                    &[],
                )?
                .l()?;
            let values_class = env.find_class(jni_str!("android/content/ContentValues"))?;
            let values = env.new_object(&values_class, jni_sig!("()V"), &[])?;
            let put_str = |env: &mut jni::Env, values: &JObject, key: &str, val: &str| {
                let jkey = env.new_string(key)?;
                let jval = env.new_string(val)?;
                env.call_method(
                    values,
                    jni_str!("put"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
                    &[JValue::Object(&jkey), JValue::Object(&jval)],
                )?;
                Ok::<(), jni::errors::Error>(())
            };
            let put_int = |env: &mut jni::Env, values: &JObject, key: &str, val: i32| {
                let jkey = env.new_string(key)?;
                let integer_class = env.find_class(jni_str!("java/lang/Integer"))?;
                let boxed = env.new_object(
                    &integer_class,
                    jni_sig!("(I)V"),
                    &[JValue::Int(val)],
                )?;
                env.call_method(
                    values,
                    jni_str!("put"),
                    jni_sig!("(Ljava/lang/String;Ljava/lang/Integer;)V"),
                    &[JValue::Object(&jkey), JValue::Object(&boxed)],
                )?;
                Ok::<(), jni::errors::Error>(())
            };
            put_str(env, &values, "_display_name", display_name)?;
            put_str(env, &values, "mime_type", mime)?;
            put_str(env, &values, "relative_path", "Download/")?;
            put_int(env, &values, "is_pending", 1)?;

            let downloads = env.find_class(jni_str!("android/provider/MediaStore$Downloads"))?;
            let collection = env
                .get_static_field(
                    &downloads,
                    jni_str!("EXTERNAL_CONTENT_URI"),
                    jni_sig!(android.net.Uri),
                )?
                .l()?;
            let uri = env
                .call_method(
                    &resolver,
                    jni_str!("insert"),
                    jni_sig!("(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;"),
                    &[JValue::Object(&collection), JValue::Object(&values)],
                )?
                .l()?;
            if uri.is_null() {
                return Err(jni::errors::Error::NullPtr("MediaStore insert returned null"));
            }
            let stream = env
                .call_method(
                    &resolver,
                    jni_str!("openOutputStream"),
                    jni_sig!("(Landroid/net/Uri;)Ljava/io/OutputStream;"),
                    &[JValue::Object(&uri)],
                )?
                .l()?;
            if stream.is_null() {
                return Err(jni::errors::Error::NullPtr("openOutputStream returned null"));
            }
            let jbytes = env.byte_array_from_slice(&bytes)?;
            env.call_method(
                &stream,
                jni_str!("write"),
                jni_sig!("([B)V"),
                &[JValue::Object(jbytes.as_ref())],
            )?;
            env.call_method(&stream, jni_str!("flush"), jni_sig!("()V"), &[])?;
            env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[])?;

            let done = env.new_object(&values_class, jni_sig!("()V"), &[])?;
            put_int(env, &done, "is_pending", 0)?;
            let null_obj = JObject::null();
            env.call_method(
                &resolver,
                jni_str!("update"),
                jni_sig!("(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I"),
                &[
                    JValue::Object(&uri),
                    JValue::Object(&done),
                    JValue::Object(&null_obj),
                    JValue::Object(&null_obj),
                ],
            )?;
            downloads_ok = true;
            Ok(())
        });
        if downloads_ok {
            Ok(format!("Saved to Downloads as {display_name}"))
        } else {
            Err(match media_err {
                Err(e) => format!("Could not save attachment: {e}"),
                Ok(()) => "Could not save attachment: MediaStore insert failed".into(),
            })
        }
    }
}

#[cfg(target_os = "android")]
pub fn show(peer_public_key_hex: &str, _call_id: &str) {
    let name = short_display_name(peer_public_key_hex);
    if let Err(e) = imp::post(imp::INCOMING_ID, "Incoming call", &format!("{name} is calling")) {
        crate::flow_log::warn("call", format!("android incoming call notify failed: {e}"));
    }
}

#[cfg(target_os = "android")]
pub fn dismiss() {
    let _ = imp::cancel(imp::INCOMING_ID);
}

#[cfg(target_os = "android")]
pub fn notify_unlock_needed() {
    let _ = imp::post(
        imp::UNLOCK_ID,
        "Ghal Bol",
        "Enter your password to receive messages",
    );
}

#[cfg(target_os = "android")]
pub fn view_intent_url() -> Option<String> {
    imp::view_url()
}

#[cfg(target_os = "android")]
pub fn open_file(path: &str) -> Result<(), String> {
    let mime = match std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "opus" | "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    };
    imp::view_file(path, mime)
}

#[cfg(target_os = "android")]
pub fn android_files_dir() -> Result<String, String> {
    imp::files_dir()
}

#[cfg(target_os = "android")]
pub fn save_keystore_backup(json: &str) -> Result<String, String> {
    imp::save_keystore_backup(json)
}

/// Copy a local file into MediaStore Downloads (same path as keystore backup).
#[cfg(target_os = "android")]
pub fn save_file_to_downloads(path: &str, display_name: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read attachment: {e}"))?;
    let ext = std::path::Path::new(display_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "opus" | "ogg" => "audio/ogg",
        "json" => "application/json",
        _ => "application/octet-stream",
    };
    imp::save_bytes_to_downloads(&bytes, display_name, mime)
}

#[cfg(target_os = "android")]
fn short_display_name(pk: &str) -> String {
    let p = pk.trim();
    if p.len() >= 16 {
        format!("{}…", &p[..8])
    } else if p.is_empty() {
        "Contact".into()
    } else {
        p.to_string()
    }
}
