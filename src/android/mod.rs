//! The Android entry point, and what the player asks of the system through
//! JNI: the access to the files, and the file it was opened with.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use iced_winit::android::{AndroidApp, set_app};
use jni::objects::{JByteArray, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};

use crate::ApplicationArgs;
use crate::config::Config;
use crate::ui::app::App;
use crate::ui::files::is_tablature;

mod exit_reason;
pub mod journal;
mod tombstone;

/// An event loop ran in this process: winit makes one per process only.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Called by the activity when it starts, in place of `main`.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    let data = app.internal_data_path();
    // the log goes to Android and, the last lines, to a file the next start
    // reads when this one ends on screen without closing
    let ended_on_screen = data.clone().and_then(journal::start);
    if data.is_none() {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("macguitar"),
        );
    }
    // Android made the activity again in a process that ran it already (it
    // was closed, then opened again, or opened with a file): the intent is
    // handed to a new process, where the player starts afresh
    if STARTED.swap(true, Ordering::SeqCst) {
        log::info!("Activity created again in the same process: restarting");
        if let Err(err) = restart(&app) {
            log::error!("Could not restart the activity: {err}");
        }
        // handed over, not crashed
        journal::on_screen(false);
        std::process::exit(0);
    }
    if let Some(folder) = &data {
        record_crashes(folder.join(CRASH_FILE));
    }
    let panic_report = data
        .as_ref()
        .map(|folder| folder.join(CRASH_FILE))
        .and_then(|path| {
            let report = std::fs::read_to_string(&path).ok();
            let _ = std::fs::remove_file(&path);
            report
        });
    let crash_report = match (panic_report, ended_on_screen) {
        (Some(panic), Some(lines)) => Some(format!("{panic}\n\n{lines}")),
        (panic, lines) => panic.or(lines),
    };
    if let Some(folder) = &data {
        Config::use_folder(folder.clone());
    }
    let _ = JNI.set(Jni {
        vm: app.vm_as_ptr() as usize,
        activity: app.activity_as_ptr() as usize,
    });
    set_app(app);
    log::info!(
        "MacGuitar {} on {}",
        env!("CARGO_PKG_VERSION"),
        device().unwrap_or_else(|err| format!("an unknown device ({err})"))
    );
    // what Android knows of the last end: the library a native crash was in
    let system_report = data.as_ref().and_then(|folder| {
        with_activity(|env, activity| exit_reason::last_crash(env, activity, folder))
            .unwrap_or_else(|err| {
                log::warn!("Could not read why the app last ended: {err}");
                None
            })
    });
    let crash_report = match (crash_report, system_report) {
        (Some(ours), Some(system)) => Some(format!("{ours}\n\n{system}")),
        (ours, system) => ours.or(system),
    };
    // the whole report kept in the downloads, to be sent on: the app may
    // fall again before it is read
    let crash_report = crash_report.map(|report| {
        let path = crate::platform::default_folder().join(CRASH_COPY);
        match std::fs::write(&path, &report) {
            Ok(()) => format!("Saved as Download/{CRASH_COPY}.\n\n{report}"),
            Err(err) => {
                log::warn!("Could not save the report in the downloads: {err}");
                report
            }
        }
    });

    // a tab given by "Open with" is copied in, where the player can read it
    let opened = data
        .as_ref()
        .and_then(|folder| match opened_file(&folder.join("opened")) {
            Ok(path) => path,
            Err(err) => {
                log::warn!("Could not read the file the app was opened with: {err}");
                None
            }
        });
    let local_config = Config::read_config().unwrap_or_else(|err| {
        log::error!("Could not read the settings: {err}");
        Config::default()
    });
    let started = App::start(ApplicationArgs {
        sound_font_bank: None,
        tab_file_path: opened,
        // multisampling is a weak spot of phone drivers (Mali), and a phone's
        // screen is dense enough without it
        no_antialiasing: true,
        theme: None,
        local_config,
        crash_report,
    });
    if let Err(err) = started {
        log::error!("{err}");
    }
    // closed as it should: a panic caught on the way (the audio thread goes
    // on after one) is no crash to report
    if let Some(folder) = &data {
        let _ = std::fs::remove_file(folder.join(CRASH_FILE));
    }
    journal::on_screen(false);
    // the activity is gone: so is the process, and the next start is clean
    std::process::exit(0);
}

/// The copy of a crash report left in the downloads, to send on.
const CRASH_COPY: &str = "MacGuitar-crash.txt";

/// Where the reason of a crash waits for the next start.
const CRASH_FILE: &str = "last-crash.txt";

/// Log a panic, and keep it in `path` to show it when the app starts again:
/// a phone shows no console.
fn record_crashes(path: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let report = format!("{info}\n\n{backtrace}");
        log::error!("{report}");
        journal::keep(format!("panic: {info}"));
        let _ = std::fs::write(&path, info.to_string());
        default(info);
    }));
}

/// Start the activity of `app` again with its intent, in a new task, for
/// the process to be replaced.
fn restart(app: &AndroidApp) -> jni::errors::Result<()> {
    // SAFETY: as in `with_activity`, for the activity just created
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let mut env = vm.attach_current_thread()?;
    let intent = env
        .call_method(&activity, "getIntent", "()Landroid/content/Intent;", &[])?
        .l()?;
    let relaunch = env.new_object(
        "android/content/Intent",
        "(Landroid/content/Intent;)V",
        &[JValue::Object(&intent)],
    )?;
    // a new task, clear of the old one; a file given keeps its access
    const FLAG_GRANT_READ_URI_PERMISSION: i32 = 0x0000_0001;
    const FLAG_ACTIVITY_NEW_TASK: i32 = 0x1000_0000;
    const FLAG_ACTIVITY_CLEAR_TASK: i32 = 0x0000_8000;
    env.call_method(
        &relaunch,
        "addFlags",
        "(I)Landroid/content/Intent;",
        &[JValue::Int(
            FLAG_ACTIVITY_NEW_TASK | FLAG_ACTIVITY_CLEAR_TASK | FLAG_GRANT_READ_URI_PERMISSION,
        )],
    )?;
    env.call_method(
        &activity,
        "startActivity",
        "(Landroid/content/Intent;)V",
        &[JValue::Object(&relaunch)],
    )?;
    Ok(())
}

/// The Java VM and the activity, kept from `android_main` as addresses.
struct Jni {
    vm: usize,
    activity: usize,
}

static JNI: OnceLock<Jni> = OnceLock::new();

/// Run `call` with a JNI environment and the activity (not the application
/// context: only an activity starts other activities and has the intent).
fn with_activity<T>(
    call: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>,
) -> jni::errors::Result<T> {
    let jni = JNI.get().ok_or(jni::errors::Error::NullPtr("activity"))?;
    // SAFETY: android-activity keeps the VM and a global reference to the
    // activity alive while the app runs; the reference is never deleted here
    let vm = unsafe { JavaVM::from_raw(jni.vm as *mut jni::sys::JavaVM) }?;
    let activity = unsafe { JObject::from_raw(jni.activity as jni::sys::jobject) };
    let mut env = vm.attach_current_thread()?;
    // the thread of android_main stays attached for the life of the app: the
    // local references a call makes (a class looked up, a string) are only
    // let go of with a frame of their own, or they pile up until the VM
    // aborts the app
    env.with_local_frame(32, |env| {
        let result = call(env, &activity);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    })
}

/// The phone and its Android, for the journal: "Pixel 7, Android 15 (API 35)".
fn device() -> jni::errors::Result<String> {
    with_activity(|env, _| {
        let text = |env: &mut JNIEnv, class: &str, field: &str| -> jni::errors::Result<String> {
            let value = env
                .get_static_field(class, field, "Ljava/lang/String;")?
                .l()?;
            Ok(env.get_string(&JString::from(value))?.into())
        };
        let model = text(env, "android/os/Build", "MODEL")?;
        let maker = text(env, "android/os/Build", "MANUFACTURER")?;
        let release = text(env, "android/os/Build$VERSION", "RELEASE")?;
        let api = env
            .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
            .i()?;
        Ok(format!("{maker} {model}, Android {release} (API {api})"))
    })
}

/// Whether the app holds the access to all files (Android 11 and later).
pub fn has_storage_access() -> bool {
    with_activity(|env, _| {
        env.call_static_method(
            "android/os/Environment",
            "isExternalStorageManager",
            "()Z",
            &[],
        )?
        .z()
    })
    .unwrap_or_else(|err| {
        log::warn!("Could not check the access to the files: {err}");
        false
    })
}

/// Put the app's task behind the others, without closing the activity.
pub fn send_to_background() {
    let moved = with_activity(|env, activity| {
        env.call_method(activity, "moveTaskToBack", "(Z)Z", &[JValue::Bool(1)])?;
        Ok(())
    });
    if let Err(err) = moved {
        log::warn!("Could not go back: {err}");
    }
}

/// Open the settings page granting this app the access to all files.
pub fn request_storage_access() {
    let opened = with_activity(|env, activity| {
        let package = env
            .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
            .l()?;
        let prefix = env.new_string("package:")?;
        let address = env
            .call_method(
                &prefix,
                "concat",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&package)],
            )?
            .l()?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&address)],
            )?
            .l()?;
        let action = env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(&action), JValue::Object(&uri)],
        )?;
        env.call_method(
            activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    });
    if let Err(err) = opened {
        log::warn!("Could not open the settings: {err}");
    }
}

/// The tab the activity was opened with ("Open with"), copied into
/// `folder`; `None` when it was opened from the launcher.
fn opened_file(folder: &Path) -> jni::errors::Result<Option<PathBuf>> {
    with_activity(|env, activity| {
        let intent = env
            .call_method(activity, "getIntent", "()Landroid/content/Intent;", &[])?
            .l()?;
        if intent.is_null() {
            return Ok(None);
        }
        let uri = env
            .call_method(&intent, "getData", "()Landroid/net/Uri;", &[])?
            .l()?;
        if uri.is_null() {
            return Ok(None);
        }
        let resolver = env
            .call_method(
                activity,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )?
            .l()?;
        let Some(name) = display_name(env, &resolver, &uri)? else {
            return Ok(None);
        };
        if !is_tablature(Path::new(&name)) {
            log::warn!("Opened with {name}, which is not a tab");
            return Ok(None);
        }
        let contents = read_all(env, &resolver, &uri)?;
        let path = folder.join(&name);
        let written =
            std::fs::create_dir_all(folder).and_then(|()| std::fs::write(&path, contents));
        match written {
            Ok(()) => Ok(Some(path)),
            Err(err) => {
                log::warn!("Could not copy {name}: {err}");
                Ok(None)
            }
        }
    })
}

/// The name of the document at `uri`, as the provider shows it, or the
/// end of its address.
fn display_name(
    env: &mut JNIEnv,
    resolver: &JObject,
    uri: &JObject,
) -> jni::errors::Result<Option<String>> {
    let null = JObject::null();
    let cursor = env
        .call_method(
            resolver,
            "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[
                JValue::Object(uri),
                JValue::Object(&null),
                JValue::Object(&null),
                JValue::Object(&null),
                JValue::Object(&null),
            ],
        )?
        .l()?;
    if !cursor.is_null() {
        let mut name = None;
        if env.call_method(&cursor, "moveToFirst", "()Z", &[])?.z()? {
            let column = env.new_string("_display_name")?;
            let index = env
                .call_method(
                    &cursor,
                    "getColumnIndex",
                    "(Ljava/lang/String;)I",
                    &[JValue::Object(&column)],
                )?
                .i()?;
            if index >= 0 {
                let value = env
                    .call_method(
                        &cursor,
                        "getString",
                        "(I)Ljava/lang/String;",
                        &[JValue::Int(index)],
                    )?
                    .l()?;
                if !value.is_null() {
                    name = Some(env.get_string(&JString::from(value))?.into());
                }
            }
        }
        env.call_method(&cursor, "close", "()V", &[])?;
        if name.is_some() {
            return Ok(name);
        }
    }
    let segment = env
        .call_method(uri, "getLastPathSegment", "()Ljava/lang/String;", &[])?
        .l()?;
    if segment.is_null() {
        return Ok(None);
    }
    let segment: String = env.get_string(&JString::from(segment))?.into();
    Ok(segment
        .rsplit(['/', ':'])
        .next()
        .map(str::to_string)
        .filter(|name| !name.is_empty()))
}

/// The bytes of the document at `uri`.
fn read_all(env: &mut JNIEnv, resolver: &JObject, uri: &JObject) -> jni::errors::Result<Vec<u8>> {
    let stream = env
        .call_method(
            resolver,
            "openInputStream",
            "(Landroid/net/Uri;)Ljava/io/InputStream;",
            &[JValue::Object(uri)],
        )?
        .l()?;
    const CHUNK: i32 = 64 * 1024;
    let buffer: JByteArray = env.new_byte_array(CHUNK)?;
    let mut chunk = vec![0_i8; CHUNK as usize];
    let mut contents = Vec::new();
    loop {
        let read = env
            .call_method(&stream, "read", "([B)I", &[JValue::Object(&buffer)])?
            .i()?;
        if read < 0 {
            break;
        }
        let read = read as usize;
        env.get_byte_array_region(&buffer, 0, &mut chunk[..read])?;
        contents.extend(chunk[..read].iter().map(|&byte| byte as u8));
    }
    env.call_method(&stream, "close", "()V", &[])?;
    Ok(contents)
}
