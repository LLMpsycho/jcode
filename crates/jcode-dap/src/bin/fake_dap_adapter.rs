use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args_os();
    let _program = args.next();
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--target-probe")) {
        let marker = args
            .next()
            .ok_or_else(|| std::io::Error::other("missing target probe marker"))?;
        std::fs::write(marker, std::process::id().to_string())?;
        loop {
            std::thread::park();
        }
    }
    let cwd = std::env::current_dir()?;
    let log_path = cwd.join("fake-dap.log");
    append_log(&log_path, &format!("adapter_pid\t{}", std::process::id()))?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let mut seq = 1_i64;
    while let Some(request) = read_message(&mut input)? {
        let command = request["command"].as_str().unwrap_or("");
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        writeln!(
            log,
            "{}\t{}",
            command,
            request.get("arguments").unwrap_or(&Value::Null)
        )?;
        if command == "initialize" {
            respond(
                &mut output,
                &mut seq,
                &request,
                Some(json!({"supportsConfigurationDoneRequest":true})),
            )?;
        } else if command == "launch" || command == "attach" {
            if cwd.join("reject-start").exists() {
                respond_error(&mut output, &mut seq, &request, "fixture rejection")?;
                continue;
            }
            if command == "attach" {
                let pid = request["arguments"]["pid"].as_u64().unwrap_or(0);
                if pid == 0 || !process_exists(pid as u32) {
                    respond_error(&mut output, &mut seq, &request, "attach PID is not live")?;
                    continue;
                }
                append_log(&log_path, &format!("verified_attach_pid\t{pid}"))?;
            }
            if command == "launch" && cwd.join("launch-descendant").exists() {
                let child = Command::new("/bin/sleep")
                    .arg("60")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?;
                append_log(&log_path, &format!("descendant_pid\t{}", child.id()))?;
                std::mem::forget(child);
            }
            if cwd.join("hang-start").exists() {
                continue;
            }
            if cwd.join("response-first").exists() {
                respond(&mut output, &mut seq, &request, None)?;
                event(&mut output, &mut seq, "initialized", None)?;
            } else {
                event(&mut output, &mut seq, "initialized", None)?;
                respond(&mut output, &mut seq, &request, None)?;
            }
            if cwd.join("emit-lifecycle").exists() {
                event(
                    &mut output,
                    &mut seq,
                    "output",
                    Some(json!({"category":"stdout","output":"fixture output"})),
                )?;
                event(
                    &mut output,
                    &mut seq,
                    "process",
                    Some(json!({"name":"fixture","systemProcessId":999999})),
                )?;
                event(
                    &mut output,
                    &mut seq,
                    "stopped",
                    Some(json!({"reason":"entry"})),
                )?;
                event(&mut output, &mut seq, "exited", Some(json!({"exitCode":0})))?;
                event(&mut output, &mut seq, "terminated", None)?;
            }
        } else if command == "configurationDone" {
            respond(&mut output, &mut seq, &request, None)?;
        } else if command == "disconnect" {
            if cwd.join("hang-disconnect").exists() {
                loop {
                    std::thread::park();
                }
            }
            respond(&mut output, &mut seq, &request, None)?;
        } else {
            respond_error(
                &mut output,
                &mut seq,
                &request,
                "unsupported fixture command",
            )?;
        }

        if command == "initialize" && cwd.join("exit-after-initialize-open-pipes").exists() {
            let child = Command::new("/bin/sleep")
                .arg("60")
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()?;
            append_log(&log_path, &format!("pipe_holder_pid\t{}", child.id()))?;
            std::mem::forget(child);
            return Ok(());
        }
    }
    loop {
        std::thread::park();
    }
}

fn append_log(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    let mut log = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(log, "{line}")
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    i32::try_from(pid).is_ok_and(|pid| unsafe { libc::kill(pid, 0) } == 0)
}
#[cfg(not(unix))]
fn process_exists(_pid: u32) -> bool {
    true
}

fn read_message(reader: &mut impl Read) -> std::io::Result<Option<Value>> {
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        match reader.read(&mut byte)? {
            0 if header.is_empty() => return Ok(None),
            0 => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            _ => header.push(byte[0]),
        }
    }
    let text = std::str::from_utf8(&header).map_err(std::io::Error::other)?;
    let length: usize = text
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .ok_or_else(|| std::io::Error::other("missing length"))?
        .trim()
        .parse()
        .map_err(std::io::Error::other)?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(std::io::Error::other)
}
fn send(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}
fn respond(
    writer: &mut impl Write,
    seq: &mut i64,
    request: &Value,
    body: Option<Value>,
) -> std::io::Result<()> {
    let value = json!({"seq":*seq,"type":"response","request_seq":request["seq"],"success":true,"command":request["command"],"body":body});
    *seq += 1;
    send(writer, &value)
}
fn respond_error(
    writer: &mut impl Write,
    seq: &mut i64,
    request: &Value,
    message: &str,
) -> std::io::Result<()> {
    let value = json!({"seq":*seq,"type":"response","request_seq":request["seq"],"success":false,"command":request["command"],"message":message});
    *seq += 1;
    send(writer, &value)
}
fn event(
    writer: &mut impl Write,
    seq: &mut i64,
    name: &str,
    body: Option<Value>,
) -> std::io::Result<()> {
    let value = json!({"seq":*seq,"type":"event","event":name,"body":body});
    *seq += 1;
    send(writer, &value)
}
