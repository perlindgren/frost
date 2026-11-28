//! Benchmark-only inactive editor replicas. There is no audio mapping, audio
//! worker, or per-block handshake in these helpers. Stdin EOF supervises lifetime.
use super::{Scratch, model::*, read, write};
use std::{
    io::Read,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub struct GuiChild {
    child: Child,
    input: Option<ChildStdin>,
    scratch: Scratch,
    expected: String,
    ready: bool,
}
impl GuiChild {
    pub fn spawn(job: &Job, index: usize, expected: &str) -> Result<Self, String> {
        let scratch = Scratch::new()?;
        let mut gui = job.clone();
        gui.plugins = vec![job.plugins[index].clone()];
        gui.mode = Mode::InProcess;
        gui.editor_replicas = false;
        let input = scratch.0.join("gui-job.ron");
        let ready = scratch.0.join("ready.ron");
        write(&input, &gui)?;
        let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .arg("--gui-helper")
            .arg(input)
            .arg(ready)
            .arg(index.to_string())
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take();
        Ok(Self {
            child,
            input,
            scratch,
            expected: expected.into(),
            ready: false,
        })
    }
    pub fn poll(&mut self) -> Result<bool, String> {
        if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("GUI-only helper exited: {status}"));
        }
        if !self.ready {
            let path = self.scratch.0.join("ready.ron");
            if path.exists() {
                let class: Result<String, String> = read(&path)?;
                if class? != self.expected {
                    return Err("Editor replica class differs from DSP instance".into());
                }
                self.ready = true;
            }
        }
        Ok(self.ready)
    }
    pub fn request_close(&mut self) {
        drop(self.input.take());
    }
    pub fn finish(&mut self, deadline: Instant) -> Result<(), String> {
        self.request_close();
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("GUI-only helper shutdown: {status}"))
                };
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Err("GUI-only helper failed to shut down".into());
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Drop for GuiChild {
    fn drop(&mut self) {
        self.request_close();
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
pub fn watch_parent() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    thread::spawn(move || {
        let _ = std::io::stdin().read(&mut [0]);
        signal.store(true, Ordering::Release);
        thread::sleep(Duration::from_secs(2));
        // A foreign GUI callback must not keep an orphan helper alive.
        std::process::exit(1);
    });
    stop
}
pub fn publish_ready(path: &PathBuf, value: &Result<String, String>) -> Result<(), String> {
    // Publish only a complete RON document; the parent never observes a partial
    // startup result. Both paths are unique inside this helper's private folder.
    let temporary = path.with_extension("tmp");
    write(&temporary, value)?;
    std::fs::rename(temporary, path).map_err(|e| e.to_string())
}
