//! 本地翻译模型的运行时：按需起一个 `llama-server`，闲置一段时间后停掉。
//!
//! 不把 llama.cpp 编进插件：`llama-server` 用系统里装的（发行版的 llama.cpp 包），没有就用
//! 设置界面下载的官方二进制（[`downloaded_server`]），我们只管它的生命周期。
//!
//! - 第一次翻译时才启动：没打开互译、没选本地模型时，不起进程、不占端口
//! - 只监听 127.0.0.1，端口每次随机挑一个空闲的（不会和别的服务抢）
//! - 闲置 `idle_secs` 秒后停掉，把 1 GB 多的内存还给系统；下一次翻译再起
//! - 进程意外退出（被 OOM 杀掉等）时，下一次翻译自动重起
//! - 输入法自己崩了、被杀了（来不及停它）：Linux 上内核替我们给它发 SIGTERM（[`spawn_server`]），
//!   不会留下一个占着 1 GB 多内存的孤儿

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// 默认的模型文件名（腾讯 HY-MT 1.5 1.8B，Q4_K_M 量化）。
pub const MODEL_FILE: &str = "HY-MT1.5-1.8B-Q4_K_M.gguf";

/// 等模型载入的最长时间。老机器从机械硬盘读 1 GB 也够了。
const LOAD_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalConfig {
    /// GGUF 模型文件。
    pub model: PathBuf,
    /// `llama-server` 可执行文件；只写名字时在 PATH 里找。
    pub server: String,
    /// 闲置多少秒后停掉；0 表示一直开着（直到关掉互译或退出输入法）。
    pub idle_secs: u64,
}

/// 把开头的 `~/` 换成家目录（设置里常这么写）。
pub fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

impl LocalConfig {
    pub fn model_exists(&self) -> bool {
        self.model.is_file()
    }

    /// 找 llama-server：写了路径就用路径；只写名字就在 PATH 里找，再看设置界面下载的那份
    /// （[`downloaded_server`]）。找不到返回 None。
    pub fn server_path(&self) -> Option<PathBuf> {
        let name = if self.server.trim().is_empty() {
            "llama-server"
        } else {
            self.server.trim()
        };
        if name.contains('/') {
            let path = PathBuf::from(name);
            return is_executable(&path).then_some(path);
        }
        find_server(name, std::env::var_os("PATH"), downloaded_server())
    }
}

/// 在 PATH 里找 name，找不到就用下载的那份（要能执行）。
fn find_server(
    name: &str,
    path: Option<std::ffi::OsString>,
    downloaded: Option<PathBuf>,
) -> Option<PathBuf> {
    path.and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|p| is_executable(p))
    })
    .or_else(|| downloaded.filter(|p| is_executable(p)))
}

/// 设置界面「下载」放 llama-server 的地方：`$XDG_DATA_HOME/wowtype/llama/llama-server`
/// （llama.cpp 官方发布的二进制，和它的动态库在同一个目录）。
pub fn downloaded_server() -> Option<PathBuf> {
    let data = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/share"),
    };
    Some(data.join("wowtype/llama/llama-server"))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

struct Running {
    child: Child,
    port: u16,
    config: LocalConfig,
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    last_used: Option<Instant>,
    /// 正在用它的请求数；不为 0 时不会因为闲置被停掉。
    busy: usize,
    /// 换配置、退出时置位，让看门狗线程退出。
    generation: u64,
    /// 有线程正在等模型载入；别的线程等它载完再用。
    starting: bool,
}

/// 一个 llama-server 进程的管理者。可以在多个线程里共享。
pub struct LocalServer {
    state: Arc<(Mutex<State>, Condvar)>,
}

impl Default for LocalServer {
    fn default() -> Self {
        Self::new()
    }
}

/// 一次请求期间持有；放掉时记下「最后用过」的时间。
pub struct Lease {
    state: Arc<(Mutex<State>, Condvar)>,
    pub base_url: String,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.state;
        if let Ok(mut s) = lock.lock() {
            s.busy = s.busy.saturating_sub(1);
            s.last_used = Some(Instant::now());
        }
        cvar.notify_all();
    }
}

impl LocalServer {
    pub fn new() -> Self {
        Self {
            state: Arc::new((Mutex::new(State::default()), Condvar::new())),
        }
    }

    /// 现在有没有在跑（测试和设置界面用）。
    pub fn is_running(&self) -> bool {
        let mut s = self.state.0.lock().unwrap();
        alive(&mut s)
    }

    /// 进程号（测试用）。
    pub fn pid(&self) -> Option<u32> {
        let s = self.state.0.lock().unwrap();
        s.running.as_ref().map(|r| r.child.id())
    }

    /// 确保服务在跑，返回 OpenAI 格式的地址（`http://127.0.0.1:端口/v1`）。
    ///
    /// 会阻塞到模型载入完（第一次要几秒），只能在后台线程里调。
    pub fn acquire(&self, config: &LocalConfig) -> Result<Lease, String> {
        let (lock, cvar) = &*self.state;
        let mut s = lock.lock().unwrap();
        while s.starting {
            s = cvar.wait(s).unwrap();
        }
        if s.running.as_ref().is_some_and(|r| &r.config != config) {
            stop(&mut s);
        }
        if !alive(&mut s) {
            if !config.model_exists() {
                return Err(format!(
                    "翻译失败：还没下载本地模型（{}）",
                    config.model.display()
                ));
            }
            let server = config.server_path().ok_or_else(|| {
                "翻译失败：没找到 llama-server（在设置的「翻译」页点下载，或者自己装 llama.cpp）"
                    .to_string()
            })?;
            // 进程一起来（QUICK_EXIT 以内）就退出，多半是端口被别人抢了（free_port 放掉到
            // llama-server 绑上之间）：换个端口马上再试，最多 3 次。载入了一阵才退出的（模型坏了、
            // 版本不对）重试也没用，还要再等一遍载入，直接报错
            let mut attempt = 0;
            loop {
                attempt += 1;
                let port = free_port().ok_or("翻译失败：找不到空闲端口")?;
                let launched = Instant::now();
                let child = spawn_retrying_busy(|| {
                    let mut command = Command::new(&server);
                    command
                        .arg("-m")
                        .arg(&config.model)
                        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
                        // 一句话用不了多少上下文；一个并发槽，内存最省
                        .args(["-c", "2048", "-np", "1"])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null());
                    spawn_server(command)
                })
                .map_err(|e| format!("翻译失败：启动 llama-server 失败（{e}）"))?;
                s.running = Some(Running {
                    child,
                    port,
                    config: config.clone(),
                });
                s.generation += 1;
                s.starting = true;
                let generation = s.generation;
                drop(s);
                // 等模型载入：不拿着锁，免得插件线程的 poll / 配置被卡住
                let ready = wait_healthy(port, LOAD_TIMEOUT, || {
                    let mut s = lock.lock().unwrap();
                    s.generation == generation && alive(&mut s)
                });
                s = lock.lock().unwrap();
                s.starting = false;
                cvar.notify_all();
                if s.generation != generation {
                    return Err("翻译已取消".to_string());
                }
                match ready {
                    Ok(()) => {
                        if config.idle_secs > 0 {
                            spawn_watchdog(
                                Arc::clone(&self.state),
                                generation,
                                Duration::from_secs(config.idle_secs),
                            );
                        }
                        break;
                    }
                    Err(message) => {
                        let quick_exit = !alive(&mut s) && launched.elapsed() < QUICK_EXIT;
                        stop(&mut s);
                        if quick_exit && attempt < 3 {
                            continue;
                        }
                        return Err(message);
                    }
                }
            }
        }
        let port = s.running.as_ref().map(|r| r.port).unwrap_or(0);
        s.busy += 1;
        s.last_used = Some(Instant::now());
        cvar.notify_all();
        Ok(Lease {
            state: Arc::clone(&self.state),
            base_url: format!("http://127.0.0.1:{port}/v1"),
        })
    }

    /// 马上停掉（关掉互译、换了模型、退出输入法时）。
    pub fn shutdown(&self) {
        let (lock, cvar) = &*self.state;
        let mut s = lock.lock().unwrap();
        stop(&mut s);
        cvar.notify_all();
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn alive(s: &mut State) -> bool {
    match s.running.as_mut() {
        Some(r) => match r.child.try_wait() {
            Ok(None) => true,
            _ => {
                s.running = None;
                false
            }
        },
        None => false,
    }
}

fn stop(s: &mut State) {
    s.generation += 1;
    if let Some(mut r) = s.running.take() {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
}

/// llama-server 起来这么久以内就退出，当作端口被抢了、换端口重试；更晚才退出的是真起不来。
/// 绑端口在载入模型之前，绑不上的话一般几十毫秒就退出；留到 3 秒是给很忙的机器（CI 上测试并行时
/// 假服务的 Python 启动就要一两秒）。比这更晚才退出的，重试只会再等一遍载入。
const QUICK_EXIT: Duration = Duration::from_secs(3);

fn free_port() -> Option<u16> {
    // 让系统挑一个；放掉以后马上给 llama-server 用。极少数情况下会被别人抢走，
    // 那样 llama-server 起来就退出，acquire 换个端口马上重试（见 QUICK_EXIT）。
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    listener.local_addr().ok().map(|a| a.port())
}

/// 轮询 `/health`，直到返回 200（模型载入完）。keep_going 返回 false 时提前放弃。
fn wait_healthy(
    port: u16,
    timeout: Duration,
    mut keep_going: impl FnMut() -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !keep_going() {
            return Err("翻译失败：llama-server 启动后退出了（模型文件对吗？）".to_string());
        }
        if health(port) == Some(200) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err("翻译失败：本地模型载入超时".to_string())
}

/// GET /health 的状态码；连不上返回 None。只发最简单的 HTTP/1.0 请求，不需要 curl。
pub fn health(port: u16) -> Option<u16> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .ok()?;
    let mut head = [0u8; 64];
    let n = stream.read(&mut head).ok()?;
    let line = std::str::from_utf8(&head[..n]).ok()?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// 看门狗：闲置超过 idle 就停掉。配置换了（generation 变了）就退出。
fn spawn_watchdog(state: Arc<(Mutex<State>, Condvar)>, generation: u64, idle: Duration) {
    thread::Builder::new()
        .name("wowtype-llama-idle".into())
        .spawn(move || {
            let (lock, cvar) = &*state;
            let mut s = lock.lock().unwrap();
            loop {
                if s.generation != generation || s.running.is_none() {
                    return;
                }
                let wait = if s.busy > 0 {
                    idle
                } else {
                    let since = s.last_used.map(|t| t.elapsed()).unwrap_or(idle);
                    if since >= idle {
                        stop(&mut s);
                        return;
                    }
                    idle - since
                };
                s = cvar.wait_timeout(s, wait).unwrap().0;
            }
        })
        .ok();
}

/// 起进程，碰上 ETXTBSY（Text file busy）就稍等重试几次。刚写好的可执行文件（刚下载的
/// llama-server、测试写的假服务）还被别的线程 fork 出来、没来得及 exec 的子进程拿着写句柄时，
/// exec 它会报这个错；那个子进程一 exec 句柄就关了。
fn spawn_retrying_busy(
    mut spawn: impl FnMut() -> std::io::Result<std::process::Child>,
) -> std::io::Result<std::process::Child> {
    const ETXTBSY: i32 = 26; // Linux 和 macOS 都是 26
    let mut tries = 0;
    loop {
        match spawn() {
            Err(e) if e.raw_os_error() == Some(ETXTBSY) && tries < 10 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            other => return other,
        }
    }
}

/// 起 llama-server，并且让它跟着我们这个进程一起死。
///
/// 正常退出、关掉互译时 [`LocalServer::shutdown`] 会停它；输入法崩了、被 `kill -9`、被 OOM 杀掉时
/// 来不及，它就成了孤儿，占着 1 GB 多内存，下次启动又起一个。Linux 上用 `PR_SET_PDEATHSIG`
/// 让内核在父进程没了的时候给它发 SIGTERM。
///
/// 内核说的「父进程没了」其实是「fork 它的那个线程没了」（见 prctl(2)）：直接在调 `acquire` 的线程里
/// fork，那个翻译线程译完一句就结束，llama-server 跟着被杀，每翻译一次都要重新载入模型。所以
/// 交给一个活到进程结束的线程去 fork。那个线程起不来就照以前那样直接起（不跟着死）。
#[cfg(target_os = "linux")]
fn spawn_server(command: Command) -> std::io::Result<Child> {
    pdeathsig::spawn(command)
}

/// macOS 没有 `PR_SET_PDEATHSIG` 这样的机制，照原样起。
#[cfg(not(target_os = "linux"))]
fn spawn_server(mut command: Command) -> std::io::Result<Child> {
    command.spawn()
}

#[cfg(target_os = "linux")]
mod pdeathsig {
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::sync::{mpsc, Mutex, OnceLock};

    type Job = (Command, mpsc::Sender<io::Result<Child>>);

    /// 专门 fork 子进程的线程：收到一个 `Command` 就起，把结果送回去。发送端放在 static 里
    /// 永远不关，所以它活到进程结束。起不了线程返回 None。
    fn spawner() -> Option<&'static Mutex<mpsc::Sender<Job>>> {
        static SPAWNER: OnceLock<Option<Mutex<mpsc::Sender<Job>>>> = OnceLock::new();
        SPAWNER
            .get_or_init(|| {
                let (tx, rx) = mpsc::channel::<Job>();
                std::thread::Builder::new()
                    .name("wowtype-llama-spawn".into())
                    .spawn(move || {
                        for (mut command, reply) in rx {
                            let _ = reply.send(spawn_tied(&mut command));
                        }
                    })
                    .ok()
                    .map(|_| Mutex::new(tx))
            })
            .as_ref()
    }

    pub fn spawn(mut command: Command) -> io::Result<Child> {
        let Some(spawner) = spawner() else {
            return command.spawn();
        };
        let (reply, result) = mpsc::channel();
        let sent = spawner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .send((command, reply));
        match sent {
            Ok(()) => result.recv().unwrap_or_else(|_| {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    "起 llama-server 的线程没了",
                ))
            }),
            // 那个线程已经不在了：把 Command 拿回来自己起
            Err(mpsc::SendError((mut command, _))) => command.spawn(),
        }
    }

    /// 在当前线程里 fork：子进程 exec 之前登记「父亲没了就给我发 SIGTERM」。
    fn spawn_tied(command: &mut Command) -> io::Result<Child> {
        // SAFETY: getpid 没有前提条件。
        let parent = unsafe { libc::getpid() };
        // SAFETY: 闭包在 fork 之后、exec 之前的子进程里跑，那时只能做 async-signal-safe 的事
        // （别的线程可能正拿着分配器的锁）：这里只有 prctl、getppid 两个系统调用，出错时
        // 用错误号构造 io::Error，都不分配内存、不拿锁。
        unsafe {
            command.pre_exec(move || {
                // 登记失败就算了（照以前那样起），不为这个翻译不了
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong);
                // 登记只管以后：父进程要是在 fork 之后、登记之前就没了，信号不会再来，
                // 这时我们已经被别的进程收养，getppid 变了。那就别起了
                if libc::getppid() != parent {
                    return Err(io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
        command.spawn()
    }
}

/// 测试用的假 llama-server（translate 的测试也用）。
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    /// 一个 Python 小服务：/health 先装 0.3 秒在载入（503）再回 200；
    /// /v1/chat/completions 按 OpenAI 格式流式返回，提示词里要求译成英语就回 Hello，否则回「你好」。
    pub fn fake_server(dir: &Path) -> PathBuf {
        let path = dir.join("fake-llama-server");
        std::fs::write(
            &path,
            r#"#!/usr/bin/env python3
import sys, time, json, http.server
port = int(sys.argv[sys.argv.index("--port") + 1])
start = time.time()
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        code = 200 if time.time() - start > 0.3 else 503
        self.send_response(code); self.end_headers(); self.wfile.write(b"{}")
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        prompt = body["messages"][-1]["content"]
        reply = "Hello" if "翻译为英语" in prompt else "你好"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream"); self.end_headers()
        for piece in (reply[:2], reply[2:]):
            chunk = {"choices": [{"delta": {"content": piece}}]}
            self.wfile.write(("data: " + json.dumps(chunk) + "\n\n").encode())
        self.wfile.write(b"data: [DONE]\n\n")
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_server(dir: &Path) -> PathBuf {
        super::test_support::fake_server(dir)
    }

    #[test]
    fn expand_home_only_touches_tilde_slash() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_home("~/m.gguf"), PathBuf::from(&home).join("m.gguf"));
        assert_eq!(expand_home("/a/~/b"), PathBuf::from("/a/~/b"));
        assert_eq!(expand_home("~x"), PathBuf::from("~x"));
    }

    fn setup(idle_secs: u64) -> (tempdir::Dir, LocalConfig) {
        let dir = tempdir::Dir::new();
        let server = fake_server(dir.path());
        let model = dir.path().join("model.gguf");
        std::fs::write(&model, b"gguf").unwrap();
        let config = LocalConfig {
            model,
            server: server.to_string_lossy().into_owned(),
            idle_secs,
        };
        (dir, config)
    }

    /// 测试用的临时目录（不引入 tempfile 依赖）。
    mod tempdir {
        pub struct Dir(std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static N: AtomicUsize = AtomicUsize::new(0);
                let p = std::env::temp_dir().join(format!(
                    "wowtype-llama-test-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::SeqCst)
                ));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn python_available() -> bool {
        Command::new("python3")
            .arg("-c")
            .arg("")
            .status()
            .is_ok_and(|s| s.success())
    }

    #[test]
    fn starts_on_demand_and_stops_when_idle() {
        if !python_available() {
            return;
        }
        let (_dir, config) = setup(1);
        let server = LocalServer::new();
        assert!(!server.is_running(), "没翻译之前不起进程");
        let lease = server.acquire(&config).unwrap();
        assert!(lease.base_url.starts_with("http://127.0.0.1:"));
        assert!(server.is_running());
        let pid = server.pid().unwrap();
        // 用着的时候不停
        thread::sleep(Duration::from_millis(1300));
        assert!(server.is_running(), "请求没结束不该停");
        drop(lease);
        // 再复用同一个进程
        let lease = server.acquire(&config).unwrap();
        assert_eq!(server.pid(), Some(pid), "还在跑就复用");
        drop(lease);
        let deadline = Instant::now() + Duration::from_secs(30);
        while server.is_running() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(100));
        }
        assert!(!server.is_running(), "闲置 1 秒后应该停掉");
        // 停了以后下一次翻译重新起
        let _lease = server.acquire(&config).unwrap();
        assert!(server.is_running());
        assert_ne!(server.pid(), Some(pid));
    }

    #[test]
    fn restarts_after_crash_and_shutdown_kills() {
        if !python_available() {
            return;
        }
        let (_dir, config) = setup(0);
        let server = LocalServer::new();
        drop(server.acquire(&config).unwrap());
        let pid = server.pid().unwrap();
        Command::new("kill").arg(pid.to_string()).status().unwrap();
        thread::sleep(Duration::from_millis(200));
        drop(server.acquire(&config).unwrap());
        assert_ne!(server.pid(), Some(pid), "进程没了就重起");
        server.shutdown();
        assert!(!server.is_running());
    }

    #[test]
    fn server_that_exits_reports_after_retries() {
        // 起来就退出（端口被抢、模型文件不对）：换端口试 3 次，还不行才报错，不会卡住
        let (dir, mut config) = setup(0);
        let (script, launches) = counting_script(dir.path(), "exits", "exit 1");
        config.server = script.display().to_string();
        let server = LocalServer::new();
        let started = Instant::now();
        let err = server.acquire(&config).err().unwrap();
        assert!(err.contains("启动后退出了"), "{err}");
        assert!(!server.is_running());
        assert_eq!(launch_count(&launches), 3);
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn server_that_dies_after_loading_is_not_retried() {
        // 载入了一阵才退出（模型坏了、版本不对）：重试也一样，只起一次
        let (dir, mut config) = setup(0);
        let (script, launches) = counting_script(dir.path(), "dies-late", "sleep 4; exit 1");
        config.server = script.display().to_string();
        let server = LocalServer::new();
        let err = server.acquire(&config).err().unwrap();
        assert!(err.contains("启动后退出了"), "{err}");
        assert_eq!(launch_count(&launches), 1);
    }

    #[test]
    fn retries_after_a_quick_exit_and_then_serves() {
        // 第一次起来就退出（像端口被抢），第二次正常：换端口重试后能用
        if !python_available() {
            return;
        }
        let (dir, mut config) = setup(0);
        let real = fake_server(dir.path());
        let (script, launches) = counting_script(
            dir.path(),
            "flaky",
            &format!(
                "[ \"$(wc -l < \"$0.launches\")\" -le 1 ] && exit 1; exec {} \"$@\"",
                real.display()
            ),
        );
        config.server = script.display().to_string();
        let server = LocalServer::new();
        drop(server.acquire(&config).unwrap());
        assert_eq!(launch_count(&launches), 2);
        assert!(server.is_running());
        server.shutdown();
    }

    /// 每次启动往 `<脚本>.launches` 里记一行再跑 body 的 shell 脚本。
    fn counting_script(dir: &Path, name: &str, body: &str) -> (PathBuf, PathBuf) {
        let script = dir.join(name);
        let launches = dir.join(format!("{name}.launches"));
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho x >> \"$0.launches\"\n{body}\n"),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, launches)
    }

    fn launch_count(launches: &Path) -> usize {
        std::fs::read_to_string(launches).map_or(0, |s| s.lines().count())
    }

    #[test]
    fn missing_model_or_server() {
        let (_dir, mut config) = setup(0);
        let server = LocalServer::new();
        let mut missing = config.clone();
        missing.model = PathBuf::from("/nonexistent/model.gguf");
        assert!(server.acquire(&missing).err().unwrap().contains("还没下载"));
        config.server = "/nonexistent/llama-server".to_string();
        assert!(server
            .acquire(&config)
            .err()
            .unwrap()
            .contains("没找到 llama-server"));
        assert!(!server.is_running());
    }

    #[test]
    fn server_lookup_prefers_path_then_downloaded() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("wowtype-llama-find-{}", std::process::id()));
        let exe = |sub: &str| {
            let d = dir.join(sub);
            std::fs::create_dir_all(&d).unwrap();
            let p = d.join("llama-server");
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let system = exe("bin");
        let downloaded = exe("data");
        let path = Some(dir.join("bin").into_os_string());
        let empty = Some(dir.join("nowhere").into_os_string());
        assert_eq!(
            find_server("llama-server", path, Some(downloaded.clone())),
            Some(system)
        );
        assert_eq!(
            find_server("llama-server", empty.clone(), Some(downloaded.clone())),
            Some(downloaded.clone())
        );
        assert_eq!(find_server("llama-server", None, Some(downloaded.clone())), Some(downloaded));
        // 下载的那份不能执行（没下完、被删）时当作没有
        assert_eq!(find_server("llama-server", empty, Some(dir.join("data/none"))), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 进程还在不在（僵尸也算不在了：收养它的进程不一定马上收尸）。
    #[cfg(target_os = "linux")]
    fn process_gone(pid: u32) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // 格式是 `pid (名字) 状态 ...`，名字里可以有括号，从最后一个右括号往后看
            Ok(stat) => stat
                .rsplit(')')
                .next()
                .is_some_and(|rest| rest.trim_start().starts_with(['Z', 'X'])),
            Err(_) => true,
        }
    }

    #[cfg(target_os = "linux")]
    fn sleeper() -> Command {
        let mut command = Command::new("sleep");
        command.arg("60").stdin(Stdio::null()).stdout(Stdio::null());
        command
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn server_outlives_the_thread_that_started_it() {
        // PR_SET_PDEATHSIG 认的是 fork 它的那个线程：翻译线程译完一句就结束，
        // llama-server 不能跟着没了（不然每翻译一次都要重新载入模型）
        let mut child = thread::spawn(|| spawn_server(sleeper()).unwrap())
            .join()
            .unwrap();
        thread::sleep(Duration::from_millis(300));
        let alive = matches!(child.try_wait(), Ok(None));
        let _ = child.kill();
        let _ = child.wait();
        assert!(alive, "起它的线程结束了，它不该跟着退出");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn server_dies_when_our_process_is_killed() {
        // 要真的杀掉一个进程才测得出来：把这个测试程序自己再起一份，只跑这一个测试，
        // 带着这个环境变量的那份扮演输入法
        const ROLE: &str = "WOWTYPE_TEST_PDEATHSIG_PIDFILE";
        if let Some(pidfile) = std::env::var_os(ROLE) {
            let pidfile = PathBuf::from(pidfile);
            // 和翻译一样：在一个马上结束的线程里起
            let child = thread::spawn(|| spawn_server(sleeper()).unwrap())
                .join()
                .unwrap();
            let tmp = pidfile.with_extension("tmp");
            std::fs::write(&tmp, child.id().to_string()).unwrap();
            std::fs::rename(&tmp, &pidfile).unwrap();
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }

        let dir = tempdir::Dir::new();
        let pidfile = dir.path().join("pid");
        let mut host = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "llama::tests::server_dies_when_our_process_is_killed"])
            .env(ROLE, &pidfile)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let wait_for = |what: &mut dyn FnMut() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !what() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            what()
        };
        let mut pid = None;
        wait_for(&mut || {
            pid = std::fs::read_to_string(&pidfile)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            pid.is_some()
        });
        let Some(pid) = pid else {
            let _ = host.kill();
            let _ = host.wait();
            panic!("扮演输入法的那份没起来");
        };
        thread::sleep(Duration::from_millis(200));
        let alive_before = !process_gone(pid);
        // SIGKILL：来不及做任何清理
        host.kill().unwrap();
        host.wait().unwrap();
        let gone = wait_for(&mut || process_gone(pid));
        if !gone {
            let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
        }
        assert!(alive_before, "输入法还在的时候它应该在跑");
        assert!(gone, "输入法被杀以后 llama-server 应该跟着退出");
    }
}
