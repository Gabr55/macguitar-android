//! A native crash as Android records it, a tombstone in protobuf
//! (`system/core/debuggerd/proto/tombstone.proto`): the signal, the
//! crashing thread and its frames, and the last lines of the log.

/// A field of a protobuf message: an integer, or bytes (a string, a
/// message).
enum Value<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

/// The fields of a message, in order; a malformed end is left out.
fn fields(mut data: &[u8]) -> Vec<(u64, Value<'_>)> {
    let mut fields = Vec::new();
    while let Some((key, rest)) = varint(data) {
        data = rest;
        let number = key >> 3;
        match key & 7 {
            0 => {
                let Some((value, rest)) = varint(data) else {
                    break;
                };
                fields.push((number, Value::Int(value)));
                data = rest;
            }
            1 if data.len() >= 8 => data = &data[8..],
            2 => {
                let Some((length, rest)) = varint(data) else {
                    break;
                };
                let Ok(length) = usize::try_from(length) else {
                    break;
                };
                if rest.len() < length {
                    break;
                }
                fields.push((number, Value::Bytes(&rest[..length])));
                data = &rest[length..];
            }
            5 if data.len() >= 4 => data = &data[4..],
            _ => break,
        }
    }
    fields
}

fn varint(data: &[u8]) -> Option<(u64, &[u8])> {
    let mut value = 0_u64;
    for (index, &byte) in data.iter().enumerate().take(10) {
        value |= u64::from(byte & 0x7f) << (7 * index);
        if byte & 0x80 == 0 {
            return Some((value, &data[index + 1..]));
        }
    }
    None
}

fn int(fields: &[(u64, Value)], number: u64) -> Option<u64> {
    fields.iter().find_map(|(n, value)| match value {
        Value::Int(value) if *n == number => Some(*value),
        _ => None,
    })
}

fn text<'a>(fields: &[(u64, Value<'a>)], number: u64) -> Option<&'a str> {
    fields.iter().find_map(|(n, value)| match value {
        Value::Bytes(bytes) if *n == number => std::str::from_utf8(bytes).ok(),
        _ => None,
    })
}

fn messages<'a>(fields: &[(u64, Value<'a>)], number: u64) -> Vec<&'a [u8]> {
    fields
        .iter()
        .filter_map(|(n, value)| match value {
            Value::Bytes(bytes) if *n == number => Some(*bytes),
            _ => None,
        })
        .collect()
}

/// How many frames of the crashing thread are told.
const FRAMES: usize = 24;
/// How many of the last log lines are told.
const LOG_LINES: usize = 40;

/// The tombstone told in lines: the signal, why, the crashing thread with
/// its frames, then the last lines of the log. `None` when `data` is no
/// tombstone.
pub fn describe(data: &[u8]) -> Option<String> {
    let tombstone = fields(data);
    let tid = int(&tombstone, 6)?;
    let mut lines = Vec::new();

    if let Some(signal) = messages(&tombstone, 10).first() {
        let signal = fields(signal);
        let name = text(&signal, 2).unwrap_or("signal");
        let code = text(&signal, 4).unwrap_or("");
        let address = int(&signal, 9)
            .filter(|_| int(&signal, 8) == Some(1))
            .map(|address| format!(" at {address:#x}"))
            .unwrap_or_default();
        lines.push(format!("{name} {code}{address}"));
    }
    for cause in messages(&tombstone, 15) {
        if let Some(cause) = text(&fields(cause), 1) {
            lines.push(format!("Cause: {cause}"));
        }
    }
    if let Some(abort) = text(&tombstone, 14).filter(|abort| !abort.is_empty()) {
        lines.push(format!("Abort: {abort}"));
    }

    // the threads are a map: entries of a key (the id) and a thread
    let crashed = messages(&tombstone, 16).into_iter().find_map(|entry| {
        let entry = fields(entry);
        let thread = messages(&entry, 2).first().map(|thread| fields(thread))?;
        (int(&thread, 1) == Some(tid)).then_some(thread)
    });
    if let Some(thread) = crashed {
        let name = text(&thread, 2).unwrap_or("?");
        lines.push(format!("Thread \"{name}\" ({tid}):"));
        for (index, frame) in messages(&thread, 4).into_iter().take(FRAMES).enumerate() {
            let frame = fields(frame);
            let file = text(&frame, 6)
                .map(|path| path.rsplit('/').next().unwrap_or(path))
                .unwrap_or("?");
            let function = text(&frame, 4)
                .filter(|name| !name.is_empty())
                .map(|name| format!(" {:#}", rustc_demangle::demangle(name)))
                .unwrap_or_default();
            let line: String = format!("#{index:02} {file}{function}")
                .chars()
                .take(140)
                .collect();
            lines.push(line);
        }
    }

    let mut log = Vec::new();
    for buffer in messages(&tombstone, 18) {
        for message in messages(&fields(buffer), 2) {
            let message = fields(message);
            let tag = text(&message, 5).unwrap_or("");
            let body = text(&message, 6).unwrap_or("").trim_end();
            let time = text(&message, 1).unwrap_or("");
            // the time of day, without the date
            let time = time.split_whitespace().nth(1).unwrap_or(time);
            log.push(
                format!("{time} {tag}: {body}")
                    .chars()
                    .take(160)
                    .collect::<String>(),
            );
        }
    }
    if !log.is_empty() {
        lines.push("Last log:".to_string());
        let skip = log.len().saturating_sub(LOG_LINES);
        lines.extend(log.into_iter().skip(skip));
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = u8::try_from(value & 0x7f).unwrap();
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    /// A length-delimited field.
    fn field(number: u64, bytes: &[u8]) -> Vec<u8> {
        let mut out = encode((number << 3) | 2);
        out.extend(encode(bytes.len() as u64));
        out.extend_from_slice(bytes);
        out
    }

    fn int_field(number: u64, value: u64) -> Vec<u8> {
        let mut out = encode(number << 3);
        out.extend(encode(value));
        out
    }

    #[test]
    fn a_tombstone_tells_the_crashing_thread() {
        let mut frame = field(6, b"/vendor/lib64/egl/libGLES_mali.so");
        frame.extend(field(4, b"_ZN4core9panicking5panic17h0123456789abcdefE"));
        let mut thread = int_field(1, 42);
        thread.extend(field(2, b"tokio-rt-worker"));
        thread.extend(field(4, &frame));
        let mut entry = int_field(1, 42);
        entry.extend(field(2, &thread));
        let mut signal = field(2, b"SIGSEGV");
        signal.extend(field(4, b"SEGV_MAPERR"));
        let mut tombstone = int_field(6, 42);
        tombstone.extend(field(10, &signal));
        tombstone.extend(field(16, &entry));
        let told = describe(&tombstone).unwrap();
        assert_eq!(
            told,
            "SIGSEGV SEGV_MAPERR\nThread \"tokio-rt-worker\" (42):\n#00 libGLES_mali.so core::panicking::panic"
        );
    }
}
