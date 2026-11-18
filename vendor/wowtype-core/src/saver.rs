//! 学习数据在后台存盘：按键的线程只在锁里抄一份要写的数据，交给一个专门的线程去写
//! （[`crate::fsutil::write_atomic`]：先写临时文件、刷到盘上再改名，要等两次 fsync）。
//!
//! - 连着交过来的几次合并：同一个文件还没开始写时，后来的那份顶掉前面的（写的都是整份，只要最新的）；
//! - [`Saver::wait`] 等写完（进程退出、外壳析构、换词库前），[`Saver`] 被 drop 时也会先写完再退；
//! - 写文件的线程第一次交活时才起。

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Result;

/// 一份要写的数据：在写文件的线程上把抄好的数据排成文字。
pub type Job = Box<dyn FnOnce() -> Result<String> + Send>;

#[derive(Default)]
struct State {
    /// 还没开始写的，按第一次交过来的顺序；同一个文件只留最新的一份。
    pending: Vec<(PathBuf, Job)>,
    /// 写文件的线程手里正在写。
    writing: bool,
    /// 要退出了：写完手里的和排着的就退。
    stop: bool,
    /// 上次 [`Saver::wait`] 以后写失败的。
    errors: Vec<String>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    /// 有活了、要退出了。
    wake: Condvar,
    /// 写完一批了。
    idle: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // 写文件的线程不会拿着锁 panic（写文件在锁外面），真坏了也接着用里面的数据
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 等后台存盘的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    /// 都写完了，上次问过以后的都写成了。
    Done,
    /// 还没写完（等的时间到了）。
    Pending,
    /// 上次问过以后有写失败的（原因，几个文件的用「；」连起来）。只报一次。
    Failed(String),
}

/// 后台存盘。
#[derive(Default)]
pub struct Saver {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Saver {
    /// 交一份要写进 path 的数据。path 还排着没写的那份被这份顶掉。
    pub fn submit(&self, path: PathBuf, job: Job) {
        {
            let mut state = self.shared.lock();
            match state.pending.iter_mut().find(|(p, _)| *p == path) {
                Some(slot) => slot.1 = job,
                None => state.pending.push((path, job)),
            }
        }
        self.start_worker();
        self.shared.wake.notify_all();
    }

    fn start_worker(&self) {
        let mut worker = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        if worker.is_some() {
            return;
        }
        let shared = self.shared.clone();
        match std::thread::Builder::new()
            .name("wowtype-save".into())
            .spawn(move || run(&shared))
        {
            Ok(handle) => *worker = Some(handle),
            // 起不了线程（极少见）：就在这里写，和以前同步存一样
            Err(_) => run_now(&self.shared),
        }
    }

    /// 等排着的、正在写的都写完。timeout 是 None 时一直等。
    pub fn wait(&self, timeout: Option<Duration>) -> SaveStatus {
        let deadline = timeout.map(|t| Instant::now() + t);
        let mut state = self.shared.lock();
        while !state.pending.is_empty() || state.writing {
            match deadline {
                None => state = self.shared.idle.wait(state).unwrap_or_else(|e| e.into_inner()),
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return SaveStatus::Pending;
                    }
                    state = self
                        .shared
                        .idle
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            }
        }
        if state.errors.is_empty() {
            SaveStatus::Done
        } else {
            SaveStatus::Failed(std::mem::take(&mut state.errors).join("；"))
        }
    }

    /// 等写完，结果留给下一次 [`Saver::wait`]（同步存盘前用：别让排着的旧数据后写、盖掉新的）。
    pub fn flush(&self) {
        let mut state = self.shared.lock();
        while !state.pending.is_empty() || state.writing {
            state = self.shared.idle.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.wake.notify_all();
        let worker = self.worker.get_mut().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

/// 写文件的线程：有活就整批拿走写，写完叫醒等着的；要退出时把排着的写完再退。
fn run(shared: &Shared) {
    loop {
        let jobs = {
            let mut state = shared.lock();
            while state.pending.is_empty() && !state.stop {
                state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if state.pending.is_empty() {
                return;
            }
            state.writing = true;
            std::mem::take(&mut state.pending)
        };
        let errors = write_all(jobs);
        let mut state = shared.lock();
        state.writing = false;
        state.errors.extend(errors);
        shared.idle.notify_all();
    }
}

/// 起不了线程时在交活的线程上写。
fn run_now(shared: &Shared) {
    let jobs = std::mem::take(&mut shared.lock().pending);
    let errors = write_all(jobs);
    shared.lock().errors.extend(errors);
    shared.idle.notify_all();
}

fn write_all(jobs: Vec<(PathBuf, Job)>) -> Vec<String> {
    let mut errors = Vec::new();
    for (path, job) in jobs {
        // 排文字、写文件出了 panic 也别把线程弄死（以后的存盘都靠它）
        let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let text = job()?;
            crate::fsutil::write_atomic(&path, text.as_bytes())
        }));
        match written {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(format!("{}：{e:#}", path.display())),
            Err(_) => errors.push(format!("{}：存盘时出错（panic）", path.display())),
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wowtype-saver-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_in_background_and_waits() {
        let dir = dir("write");
        let saver = Saver::default();
        let path = dir.join("a.tsv");
        saver.submit(path.clone(), Box::new(|| Ok("一\n".to_string())));
        assert_eq!(saver.wait(None), SaveStatus::Done);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "一\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn later_request_replaces_one_not_yet_written() {
        let dir = dir("merge");
        let saver = Saver::default();
        let path = dir.join("a.tsv");
        let runs = Arc::new(AtomicUsize::new(0));
        // 先占住写文件的线程，后面两份排着：只写最新的那份
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let gate = Mutex::new(rx);
        saver.submit(
            dir.join("gate.tsv"),
            Box::new(move || {
                let _ = gate.lock().unwrap().recv();
                Ok(String::new())
            }),
        );
        // 等它开始写（从排着的里拿走了）
        while !saver.shared.lock().pending.is_empty() {
            std::thread::yield_now();
        }
        for text in ["旧", "新"] {
            let runs = runs.clone();
            saver.submit(
                path.clone(),
                Box::new(move || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Ok(text.to_string())
                }),
            );
        }
        assert_eq!(saver.wait(Some(Duration::ZERO)), SaveStatus::Pending);
        tx.send(()).unwrap();
        assert_eq!(saver.wait(None), SaveStatus::Done);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "新");
        assert_eq!(runs.load(Ordering::SeqCst), 1, "排着的两份合并成一次");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failures_are_reported_once() {
        let dir = dir("fail");
        let saver = Saver::default();
        // 目录不存在：写不了
        saver.submit(dir.join("没有/a.tsv"), Box::new(|| Ok("x".to_string())));
        assert!(matches!(saver.wait(None), SaveStatus::Failed(e) if e.contains("a.tsv")));
        assert_eq!(saver.wait(None), SaveStatus::Done, "只报一次");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn drop_finishes_pending_writes() {
        let dir = dir("drop");
        let path = dir.join("a.tsv");
        {
            let saver = Saver::default();
            saver.submit(
                path.clone(),
                Box::new(|| {
                    std::thread::sleep(Duration::from_millis(50));
                    Ok("写完了".to_string())
                }),
            );
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "写完了");
        std::fs::remove_dir_all(&dir).ok();
    }
}
