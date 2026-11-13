//! Bounded process I/O, deadlines and cancellation for yt-dlp and FFmpeg.
use std::os::windows::{io::AsRawHandle, process::CommandExt};
use std::{
    io::Read,
    process::{Child, Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    },
};

pub type Cancellation = Arc<AtomicBool>;
pub fn cancellation() -> Cancellation {
    Arc::new(AtomicBool::new(false))
}
pub fn check_cancel(cancel: &Cancellation) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Operație anulată".into())
    } else {
        Ok(())
    }
}
const MAX_STDOUT: usize = 4 * 1024 * 1024;
const MAX_STDERR: usize = 64 * 1024;

fn update_progress(line: &[u8], progress: &Mutex<f32>) {
    let Ok(text) = std::str::from_utf8(line) else {
        return;
    };
    let value = if text.contains("__AURA_PROCESSING__") {
        // Extraction/conversion has no trustworthy percentage.
        f32::NAN
    } else {
        let Some(value) = text
            .split_once("__AURA_PROGRESS__")
            .and_then(|(_, value)| value.split('%').next())
            .and_then(|value| value.trim().parse::<f32>().ok())
            .filter(|value| value.is_finite())
        else {
            return;
        };
        value.clamp(0.0, 100.0)
    };
    if let Ok(mut progress) = progress.lock() {
        *progress = value;
    }
}

struct Job(HANDLE);
impl Job {
    fn new() -> Result<Self, String> {
        // An unnamed, non-inheritable job; its lifetime is scoped to this run.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let job = Self(handle);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(job)
    }
    fn attach(&self, child: &Child) -> Result<(), String> {
        if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) } == 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }
    fn terminate(&self) {
        unsafe {
            TerminateJobObject(self.0, 1);
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn read_output(
    mut input: impl Read,
    limit: usize,
    tail: bool,
    overflow: &AtomicBool,
    progress: Option<Arc<Mutex<f32>>>,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut line = Vec::new();
    loop {
        let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        if tail {
            output.extend_from_slice(&buffer[..count]);
            if output.len() > limit {
                output.drain(..output.len() - limit);
            }
        } else {
            let remaining = limit.saturating_sub(output.len());
            output.extend_from_slice(&buffer[..count.min(remaining)]);
            if count > remaining {
                overflow.store(true, Ordering::Relaxed);
            }
        }
        if let Some(progress) = &progress {
            for byte in &buffer[..count] {
                if *byte == b'\n' || *byte == b'\r' {
                    update_progress(&line, progress);
                    line.clear();
                } else if line.len() < 8192 {
                    line.push(*byte);
                }
            }
        }
    }
    if let Some(progress) = &progress {
        update_progress(&line, progress);
    }
    Ok(output)
}

pub fn run(
    command: &mut Command,
    timeout: Duration,
    cancel: &Cancellation,
    progress: Option<Arc<Mutex<f32>>>,
) -> Result<Output, String> {
    check_cancel(cancel)?;
    let job = Job::new()?;
    let mut child = command
        .creation_flags(0x0800_0000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Procesul nu poate fi pornit: {error}"))?;
    if let Err(error) = job.attach(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("Procesul nu poate fi supravegheat: {error}"));
    }
    let stdout = child
        .stdout
        .take()
        .ok_or("Ieșirea procesului nu este disponibilă")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Erorile procesului nu sunt disponibile")?;
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout_overflow = overflow.clone();
    let stderr_overflow = overflow.clone();
    // yt-dlp normally emits progress templates on stdout. Some versions and
    // wrappers use stderr, so both bounded readers publish the same protocol.
    let stdout_progress = progress.clone();
    let stdout = thread::spawn(move || {
        read_output(stdout, MAX_STDOUT, false, &stdout_overflow, stdout_progress)
    });
    let stderr =
        thread::spawn(move || read_output(stderr, MAX_STDERR, true, &stderr_overflow, progress));
    let started = Instant::now();
    let status = loop {
        if let Err(error) = check_cancel(cancel) {
            break Err(error);
        }
        if started.elapsed() >= timeout {
            break Err("Operația a depășit limita de timp. Poți reîncerca.".to_owned());
        }
        if overflow.load(Ordering::Relaxed) {
            break Err("Răspunsul procesului depășește limita de 4 MiB".to_owned());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(error) => break Err(error.to_string()),
        }
        thread::sleep(Duration::from_millis(20));
    };
    // Close descendant pipe writers even if a parent exited without waiting.
    job.terminate();
    let _ = child.kill();
    let _ = child.wait();
    let stdout = stdout.join().map_err(|_| "Citirea răspunsului a eșuat")??;
    let stderr = stderr.join().map_err(|_| "Citirea erorilor a eșuat")??;
    if overflow.load(Ordering::Relaxed) {
        return Err("Răspunsul procesului depășește limita de 4 MiB".into());
    }
    Ok(Output {
        status: status?,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "process_runner::tests::process_fixture",
                "--nocapture",
            ])
            .env("S4GGZ_PROCESS_TEST_MODE", mode);
        command
    }
    #[test]
    #[ignore = "Subprocess fixture; invoked by supervisor tests"]
    fn process_fixture() {
        use std::io::Write;
        match std::env::var("S4GGZ_PROCESS_TEST_MODE").as_deref() {
            Ok("sleep") => thread::sleep(Duration::from_secs(30)),
            Ok("large") => {
                let _ = std::io::stdout().write_all(&vec![b'x'; 6 * 1024 * 1024]);
            }
            Ok("stderr") => {
                let _ = std::io::stderr().write_all(&vec![b'e'; 1024 * 1024]);
            }
            Ok("progress-stdout") => {
                let mut output = std::io::stdout().lock();
                output
                    .write_all(b"__AURA_PROGRESS__ 17.5%\r__AURA_PROGRESS__ 63.2%")
                    .unwrap();
                output.flush().unwrap();
            }
            Ok("progress-stderr") => {
                let mut output = std::io::stderr().lock();
                output
                    .write_all(b"__AURA_PROGRESS__ 17.5%\r__AURA_PROGRESS__ 63.2%")
                    .unwrap();
                output.flush().unwrap();
            }
            _ => println!("fixture-ok"),
        }
    }

    #[test]
    fn progress_parsing_handles_split_lines_eof_and_processing_without_a_percentage() {
        struct Chunked<'a>(&'a [u8]);
        impl Read for Chunked<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let count = self.0.len().min(output.len()).min(3);
                output[..count].copy_from_slice(&self.0[..count]);
                self.0 = &self.0[count..];
                Ok(count)
            }
        }
        let bytes = b"ordinary log\r\n__AURA_PROGRESS__ 12.5%\r\n__AURA_PROGRESS__ 52.4%";
        for tail in [false, true] {
            let progress = Arc::new(Mutex::new(f32::NAN));
            let overflow = AtomicBool::new(false);
            let limit = if tail { 8 } else { MAX_STDOUT };
            let output = read_output(
                Chunked(bytes),
                limit,
                tail,
                &overflow,
                Some(progress.clone()),
            )
            .unwrap();
            assert_eq!(*progress.lock().unwrap(), 52.4);
            assert!(!overflow.load(Ordering::Relaxed));
            assert!(output.len() <= limit);
            update_progress(b"__AURA_PROGRESS__ N/A", &progress);
            update_progress(b"__AURA_PROGRESS__ NaN%", &progress);
            assert_eq!(*progress.lock().unwrap(), 52.4);
            update_progress(b"__AURA_PROCESSING__", &progress);
            assert!(progress.lock().unwrap().is_nan());
        }
    }

    #[test]
    fn supervisor_reads_download_progress_from_stdout_and_stderr() {
        for mode in ["progress-stdout", "progress-stderr", "success"] {
            let progress = Arc::new(Mutex::new(f32::NAN));
            let output = run(
                &mut fixture(mode),
                Duration::from_secs(5),
                &cancellation(),
                Some(progress.clone()),
            )
            .unwrap();
            assert!(output.status.success());
            let value = *progress.lock().unwrap();
            if mode == "success" {
                assert!(
                    value.is_nan(),
                    "No progress event must not invent a percentage"
                );
            } else {
                assert_eq!(value, 63.2, "Progress was lost on {mode}");
            }
        }
    }
    #[test]
    fn supervisor_bounds_output_timeout_and_cancellation() {
        let cancel = cancellation();
        let output = run(
            &mut fixture("success"),
            Duration::from_secs(5),
            &cancel,
            None,
        )
        .unwrap();
        assert!(output.status.success());
        let output = run(
            &mut fixture("stderr"),
            Duration::from_secs(5),
            &cancel,
            None,
        )
        .unwrap();
        assert!(output.stderr.len() <= MAX_STDERR);
        assert!(
            run(&mut fixture("large"), Duration::from_secs(5), &cancel, None)
                .unwrap_err()
                .contains("4 MiB")
        );
        let started = Instant::now();
        assert!(run(
            &mut fixture("sleep"),
            Duration::from_millis(200),
            &cancel,
            None
        )
        .unwrap_err()
        .contains("timp"));
        assert!(started.elapsed() < Duration::from_secs(3));
        let cancel = cancellation();
        let trigger = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            trigger.store(true, Ordering::Relaxed);
        });
        assert!(
            run(&mut fixture("sleep"), Duration::from_secs(5), &cancel, None)
                .unwrap_err()
                .contains("anulată")
        );
    }
}
