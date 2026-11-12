use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tracing_subscriber::layer::SubscriberExt;

const MAX_LOG_LINES: usize = 5000;

pub struct LogBuffer {
    lines: Mutex<VecDeque<String>>,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            lines: Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES)),
        }
    }

    pub fn push(&self, line: String) {
        let mut lines = self.lines.lock().unwrap();
        if lines.len() >= MAX_LOG_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    pub fn snapshot(&self) -> Vec<String> {
        let lines = self.lines.lock().unwrap();
        lines.iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.lines.lock().unwrap().len()
    }
}

pub fn setup_logging(buffer: Arc<LogBuffer>, to_stdout: bool, level: &str) {
    let timer = LocalTimeFallback;

    let buffer_clone = buffer.clone();
    let ring_layer = tracing_subscriber::fmt::layer()
        .with_writer(move || RingWriter {
            buffer: buffer_clone.clone(),
        })
        .with_timer(timer)
        .with_ansi(false)
        .with_target(false);

    let env_filter = tracing_subscriber::EnvFilter::try_new(level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if to_stdout {
        let stdout_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_timer(timer)
            .with_ansi(true)
            .with_target(false);
        let subscriber = tracing_subscriber::registry()
            .with(env_filter)
            .with(ring_layer)
            .with(stdout_layer);
        tracing::subscriber::set_global_default(subscriber).ok();
    } else {
        let subscriber = tracing_subscriber::registry()
            .with(env_filter)
            .with(ring_layer);
        tracing::subscriber::set_global_default(subscriber).ok();
    }
}

struct RingWriter {
    buffer: Arc<LogBuffer>,
}

impl std::io::Write for RingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(text) = std::str::from_utf8(buf) {
            for line in text.lines() {
                self.buffer.push(line.to_string());
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Custom time formatter: tries local timezone first, falls back to UTC.
/// On WSL2 and other environments without a proper timezone database,
/// `LocalTime::rfc_3339()` produces `<unknown time>` because the `time`
/// crate cannot resolve the local offset. This formatter detects the
/// failure and falls back to UTC.
#[derive(Clone, Copy)]
struct LocalTimeFallback;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimeFallback {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        // Use chrono::Local which reads /etc/localtime properly (works on WSL2).
        // The `time` crate relies on TZ env var which WSL2 does not set.
        let now = chrono::Local::now();
        write!(w, "{}", now.format("%Y-%m-%dT%H:%M:%S%.3f%:z"))
    }
}

/// Redirect OS-level stderr (fd 2) to a pipe during `f()` so that native
/// libraries (ONNX Runtime, etc.) don't corrupt the TUI display.
/// Captured lines are forwarded to `tracing::info!` with an `[onnx]` prefix
/// so they show up in the TUI log panel.
#[cfg(unix)]
pub fn capture_native_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    use std::time::{Duration, Instant};

    let saved_stderr = unsafe { libc::dup(2) };
    if saved_stderr < 0 {
        return f();
    }

    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        unsafe { libc::close(saved_stderr) };
        return f();
    }
    let [read_fd, write_fd] = fds;

    unsafe { libc::dup2(write_fd, 2) };

    let result = f();

    unsafe {
        libc::dup2(saved_stderr, 2);
        libc::close(saved_stderr);
        libc::close(write_fd);
    }

    let mut output = Vec::new();
    // Avoid potential indefinite blocking here: child processes spawned during
    // `f()` may inherit stderr and keep the pipe writer open.
    let flags = unsafe { libc::fcntl(read_fd, libc::F_GETFL) };
    if flags >= 0 {
        let _ = unsafe { libc::fcntl(read_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    }
    let mut buf = [0u8; 4096];
    let mut idle_deadline = Instant::now() + Duration::from_millis(120);
    loop {
        let n = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n > 0 {
            output.extend_from_slice(&buf[..n as usize]);
            idle_deadline = Instant::now() + Duration::from_millis(80);
            continue;
        }
        if n == 0 {
            break;
        }
        let err = std::io::Error::last_os_error();
        let kind = err.kind();
        if kind == std::io::ErrorKind::WouldBlock && Instant::now() < idle_deadline {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        break;
    }
    unsafe { libc::close(read_fd) };

    if let Ok(text) = String::from_utf8(output) {
        for line in text.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                tracing::info!("[onnx] {}", trimmed);
            }
        }
    }

    result
}

#[cfg(windows)]
pub fn capture_native_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    extern "C" {
        fn _pipe(pipefd: *mut i32, psize: u32, textmode: i32) -> i32;
        fn _dup(fd: i32) -> i32;
        fn _dup2(fd1: i32, fd2: i32) -> i32;
        fn _close(fd: i32) -> i32;
        fn _read(fd: i32, buf: *mut u8, count: u32) -> i32;
        fn _get_osfhandle(fd: i32) -> isize;
    }

    const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4; // (DWORD)-12
    const HANDLE_FLAG_INHERIT: u32 = 1;
    extern "system" {
        fn GetStdHandle(nStdHandle: u32) -> isize;
        fn SetStdHandle(nStdHandle: u32, hHandle: isize) -> i32;
        fn SetHandleInformation(hObject: isize, dwMask: u32, dwFlags: u32) -> i32;
    }

    let saved_stderr_crt = unsafe { _dup(2) };
    if saved_stderr_crt < 0 {
        return f();
    }

    let saved_stderr_win = unsafe { GetStdHandle(STD_ERROR_HANDLE) };

    let mut fds = [0i32; 2];
    if unsafe { _pipe(fds.as_mut_ptr(), 4096, 0) } != 0 {
        unsafe { _close(saved_stderr_crt) };
        return f();
    }
    let [read_fd, write_fd] = fds;

    // Prevent child processes from inheriting pipe handles.
    // Without this, llama-server (spawned mid-capture) inherits the write end,
    // keeping it alive and causing _read to block and/or CRT assertions on exit.
    unsafe {
        SetHandleInformation(_get_osfhandle(read_fd), HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(_get_osfhandle(write_fd), HANDLE_FLAG_INHERIT, 0);
    }

    unsafe {
        _dup2(write_fd, 2);
        SetStdHandle(STD_ERROR_HANDLE, _get_osfhandle(write_fd));
    }

    let result = f();

    unsafe {
        _dup2(saved_stderr_crt, 2);
        SetStdHandle(STD_ERROR_HANDLE, saved_stderr_win);
        _close(saved_stderr_crt);
        _close(write_fd);
    }

    let mut output = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = unsafe { _read(read_fd, buf.as_mut_ptr(), buf.len() as u32) };
        if n <= 0 {
            break;
        }
        output.extend_from_slice(&buf[..n as usize]);
    }
    unsafe { _close(read_fd) };

    if let Ok(text) = String::from_utf8(output) {
        for line in text.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                tracing::info!("[onnx] {}", trimmed);
            }
        }
    }

    result
}

#[cfg(not(any(unix, windows)))]
pub fn capture_native_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    f()
}
