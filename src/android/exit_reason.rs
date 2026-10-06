//! Why the system last ended the app, as Android 11 and later keep it
//! (`ApplicationExitInfo`): for a native crash, its tombstone names the
//! signal and the library, which no log of the app can.

use std::path::Path;

use super::tombstone;
use jni::JNIEnv;
use jni::objects::{JObject, JString, JValue};

/// The exit last reported, by its time, not to report it twice.
const SEEN_FILE: &str = "last-exit-seen";
/// Most of a trace read: a tombstone is a few tens of kilobytes.
const TRACE_BYTES: usize = 512 * 1024;
/// How many lines of a trace are shown.
const TRACE_LINES: usize = 30;

/// What the last exit says, when it is a crash not reported yet.
pub fn last_crash(
    env: &mut JNIEnv,
    activity: &JObject,
    folder: &Path,
) -> jni::errors::Result<Option<String>> {
    let service = env.new_string("activity")?;
    let manager = env
        .call_method(
            activity,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service)],
        )?
        .l()?;
    let package = env
        .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
        .l()?;
    let exits = env
        .call_method(
            &manager,
            "getHistoricalProcessExitReasons",
            "(Ljava/lang/String;II)Ljava/util/List;",
            &[JValue::Object(&package), JValue::Int(0), JValue::Int(1)],
        )?
        .l()?;
    if env.call_method(&exits, "size", "()I", &[])?.i()? == 0 {
        return Ok(None);
    }
    let exit = env
        .call_method(&exits, "get", "(I)Ljava/lang/Object;", &[JValue::Int(0)])?
        .l()?;
    let timestamp = env.call_method(&exit, "getTimestamp", "()J", &[])?.j()?;
    let seen_path = folder.join(SEEN_FILE);
    let seen = std::fs::read_to_string(&seen_path)
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
        .unwrap_or(0);
    if timestamp <= seen {
        return Ok(None);
    }
    let _ = std::fs::write(&seen_path, timestamp.to_string());

    let reason = env.call_method(&exit, "getReason", "()I", &[])?.i()?;
    let Some(reason_name) = crash_reason(reason) else {
        return Ok(None);
    };
    let description = env
        .call_method(&exit, "getDescription", "()Ljava/lang/String;", &[])?
        .l()?;
    let description: String = if description.is_null() {
        String::new()
    } else {
        env.get_string(&JString::from(description))?.into()
    };
    let trace = read_trace(env, &exit)?;
    let mut report = format!("Android says: {reason_name}");
    if !description.is_empty() {
        report.push_str(&format!(" ({description})"));
    }
    // a tombstone read field by field; any other trace (an ANR's, as text)
    // by its telling strings
    let told = tombstone::describe(&trace).unwrap_or_else(|| trace_lines(&trace).join("\n"));
    if !told.is_empty() {
        report.push('\n');
        report.push_str(&told);
    }
    Ok(Some(report))
}

/// The name of an exit reason worth reporting, `None` for the ordinary ones
/// (closed, killed in the background, updated).
const fn crash_reason(reason: i32) -> Option<&'static str> {
    Some(match reason {
        2 => "ended by a signal",
        3 => "ended for lack of memory",
        4 => "crashed (Java)",
        5 => "crashed (native)",
        6 => "not responding",
        7 => "failed to start",
        9 => "used too much of the device",
        13 => "ended for another reason",
        _ => return None,
    })
}

/// The bytes of the trace of `exit`, if it has one.
fn read_trace(env: &mut JNIEnv, exit: &JObject) -> jni::errors::Result<Vec<u8>> {
    let stream = env
        .call_method(exit, "getTraceInputStream", "()Ljava/io/InputStream;", &[])?
        .l()?;
    if stream.is_null() {
        return Ok(Vec::new());
    }
    const CHUNK: i32 = 16 * 1024;
    let buffer = env.new_byte_array(CHUNK)?;
    let mut chunk = vec![0_i8; CHUNK as usize];
    let mut bytes = Vec::new();
    while bytes.len() < TRACE_BYTES {
        let read = env
            .call_method(&stream, "read", "([B)I", &[JValue::Object(&buffer)])?
            .i()?;
        if read < 0 {
            break;
        }
        let read = read as usize;
        env.get_byte_array_region(&buffer, 0, &mut chunk[..read])?;
        bytes.extend(chunk[..read].iter().map(|&byte| byte as u8));
    }
    env.call_method(&stream, "close", "()V", &[])?;
    Ok(bytes)
}

/// The telling text of a trace: a native tombstone comes as protobuf, its
/// strings (signal, abort message, the libraries and functions of the
/// crashing thread) read out of it; an ANR trace is text already.
pub fn trace_lines(trace: &[u8]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, lines: &mut Vec<String>| {
        if let Some(line) = readable(current.trim())
            && !lines.contains(&line)
        {
            lines.push(line);
        }
        current.clear();
    };
    for &byte in trace {
        if byte.is_ascii_graphic() || byte == b' ' {
            current.push(char::from(byte));
        } else {
            flush(&mut current, &mut lines);
        }
        if lines.len() == TRACE_LINES {
            return lines;
        }
    }
    flush(&mut current, &mut lines);
    lines.truncate(TRACE_LINES);
    lines
}

/// A string of a trace as a line to read, when it says where or why: a
/// library by its file name, a Rust function by its name, a signal or an
/// abort message as it is.
fn readable(text: &str) -> Option<String> {
    if text.len() < 4 {
        return None;
    }
    // a protobuf field tag may stick to the front of a string: the library
    // and the symbol are cut where they start
    if let Some(end) = text.find(".so").map(|at| at + 3) {
        let start = text[..end].rfind('/').map_or(0, |slash| slash + 1);
        return Some(text[start..end].to_string());
    }
    if let Some(start) = text.find("_RN").or_else(|| text.find("_ZN")) {
        // the mangled name ends where its alphabet does
        let mangled: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'))
            .collect();
        let symbol = format!("{:#}", rustc_demangle::demangle(&mangled));
        return Some(symbol.chars().take(LINE_CHARS).collect());
    }
    // the inner workings of the Java runtime say nothing of the crash
    if text.contains("art::") || text.contains("trampoline") {
        return None;
    }
    telling(text).then(|| text.chars().take(LINE_CHARS).collect())
}

/// Longest a line of a trace gets.
const LINE_CHARS: usize = 120;

/// Whether a string of a trace says why: a signal, an abort message, an
/// exception.
fn telling(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "sig",
        "abort",
        "fault",
        "panic",
        "exception",
        "error",
        "fatal",
        "jni",
        "cause",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_telling_strings_come_out_of_a_tombstone() {
        let mut trace = b"\x08\x05\x12\x07SIGSEGV\x1a\x02xy\x22".to_vec();
        trace.extend(b"B/vendor/lib64/egl/libGLES_mali.so\x00\x10");
        trace.extend(b"/vendor/lib64/egl/libGLES_mali.so\x00");
        trace.extend(b"just words here\x00");
        trace.extend(b"\"_ZN4core9panicking5panic17h0123456789abcdefE\x00");
        let lines = trace_lines(&trace);
        assert_eq!(
            lines,
            vec!["SIGSEGV", "libGLES_mali.so", "core::panicking::panic"]
        );
    }
}
