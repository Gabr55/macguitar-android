//! Documents opened in the apps of the phone: a PDF tab book in its PDF
//! viewer. Another app reads a file of the storage through the media
//! store, which gives it a content address to read it by.

use std::path::Path;
use std::time::{Duration, Instant};

use jni::JNIEnv;
use jni::objects::{JObject, JValue};

use super::with_activity;

/// How long the media store is waited for, a file just added.
const INDEXING: Duration = Duration::from_secs(5);

/// Open the PDF at `path` in the phone's PDF viewer.
pub fn open_pdf(path: &Path) -> Result<(), String> {
    let path_text = path.to_string_lossy().into_owned();
    let deadline = Instant::now() + INDEXING;
    let mut scanned = false;
    loop {
        let opened = with_activity(|env, activity| {
            let Some(uri) = media_uri(env, activity, &path_text)? else {
                return Ok(false);
            };
            view(env, activity, &uri, "application/pdf")?;
            Ok(true)
        })
        .map_err(|err| format!("Could not open the PDF: {err}"))?;
        if opened {
            return Ok(());
        }
        // not in the media store yet: it is asked to look, then waited for
        if !scanned {
            scan(&path_text).map_err(|err| format!("Could not index the PDF: {err}"))?;
            scanned = true;
        }
        if Instant::now() > deadline {
            return Err("The PDF is not known to the phone yet: try again in a moment".to_string());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The media store's address of the file at `path`, if it knows it.
fn media_uri<'a>(
    env: &mut JNIEnv<'a>,
    activity: &JObject,
    path: &str,
) -> jni::errors::Result<Option<JObject<'a>>> {
    let volume = env.new_string("external")?;
    let files = env
        .call_static_method(
            "android/provider/MediaStore$Files",
            "getContentUri",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&volume)],
        )?
        .l()?;
    let column = env.new_string("_id")?;
    let projection = env.new_object_array(1, "java/lang/String", &column)?;
    let selection = env.new_string("_data=?")?;
    let value = env.new_string(path)?;
    let arguments = env.new_object_array(1, "java/lang/String", &value)?;
    let resolver = env
        .call_method(
            activity,
            "getContentResolver",
            "()Landroid/content/ContentResolver;",
            &[],
        )?
        .l()?;
    let null = JObject::null();
    let cursor = env
        .call_method(
            &resolver,
            "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[
                JValue::Object(&files),
                JValue::Object(&projection),
                JValue::Object(&selection),
                JValue::Object(&arguments),
                JValue::Object(&null),
            ],
        )?
        .l()?;
    if cursor.is_null() {
        return Ok(None);
    }
    let found = env.call_method(&cursor, "moveToFirst", "()Z", &[])?.z()?;
    let id = if found {
        Some(
            env.call_method(&cursor, "getLong", "(I)J", &[JValue::Int(0)])?
                .j()?,
        )
    } else {
        None
    };
    env.call_method(&cursor, "close", "()V", &[])?;
    let Some(id) = id else {
        return Ok(None);
    };
    let uri = env
        .call_static_method(
            "android/content/ContentUris",
            "withAppendedId",
            "(Landroid/net/Uri;J)Landroid/net/Uri;",
            &[JValue::Object(&files), JValue::Long(id)],
        )?
        .l()?;
    Ok(Some(uri))
}

/// Ask the media store to look at the file at `path`.
fn scan(path: &str) -> jni::errors::Result<()> {
    with_activity(|env, activity| {
        let value = env.new_string(path)?;
        let paths = env.new_object_array(1, "java/lang/String", &value)?;
        let null = JObject::null();
        env.call_static_method(
            "android/media/MediaScannerConnection",
            "scanFile",
            "(Landroid/content/Context;[Ljava/lang/String;[Ljava/lang/String;Landroid/media/MediaScannerConnection$OnScanCompletedListener;)V",
            &[
                JValue::Object(activity),
                JValue::Object(&paths),
                JValue::Object(&null),
                JValue::Object(&null),
            ],
        )?;
        Ok(())
    })
}

/// Show `uri` of `mime` in the app the system has for it, allowed to read it.
fn view(
    env: &mut JNIEnv,
    activity: &JObject,
    uri: &JObject,
    mime: &str,
) -> jni::errors::Result<()> {
    const FLAG_GRANT_READ_URI_PERMISSION: i32 = 0x0000_0001;
    const FLAG_ACTIVITY_NEW_TASK: i32 = 0x1000_0000;
    let action = env.new_string("android.intent.action.VIEW")?;
    let intent = env.new_object(
        "android/content/Intent",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&action)],
    )?;
    let kind = env.new_string(mime)?;
    env.call_method(
        &intent,
        "setDataAndType",
        "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(uri), JValue::Object(&kind)],
    )?;
    env.call_method(
        &intent,
        "addFlags",
        "(I)Landroid/content/Intent;",
        &[JValue::Int(
            FLAG_GRANT_READ_URI_PERMISSION | FLAG_ACTIVITY_NEW_TASK,
        )],
    )?;
    env.call_method(
        activity,
        "startActivity",
        "(Landroid/content/Intent;)V",
        &[JValue::Object(&intent)],
    )?;
    Ok(())
}
