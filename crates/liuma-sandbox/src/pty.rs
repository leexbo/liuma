//! PTY spawn:portable-pty 包装 + 沙箱 argv 包装。
//!
//! 用途:需要终端语义的工具(isatty、彩色输出、行缓冲交互),以及
//! 交互式终端会话(带尺寸 spawn + master 写入口 + resize)。
//! codex 同款包法(portable-pty + 宿主侧进程管理)。
//!
//! 沙箱:portable-pty 的 CommandBuilder 不暴露 pre_exec,故沙箱只能经
//! argv 包装(bwrap / seatbelt rung)——与 pipes 路径同一探测链。
//! Landlock-only 系统(无法 argv 包装)按 fail-closed **拒绝** PTY 执行,
//! 不静默降级为无沙箱。
//!
//! 读写均为阻塞 IO,各自经 `spawn_blocking` 桥接成通道;master drop 即
//! 向会话送 SIGHUP(PTY 语义),配合 `kill` 的组信号。

use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::Write;
use thiserror::Error;

use crate::sandbox::{Rung, SandboxPolicy, wrap_argv};

/// PTY 错误
#[derive(Debug, Error)]
pub enum PtyError {
    /// PTY 系统调用失败
    #[error("pty: {0}")]
    Io(String),
    /// 沙箱拒绝(landlock-only 系统无法 argv 包装,fail-closed)
    #[error("pty sandbox: {0}")]
    Sandbox(String),
}

/// PTY 会话:子进程 + master + killer 句柄。
///
/// `wait` 消费子进程句柄(退出后 killer 仍可用于补杀);
/// `kill` 走 killer 组信号。
pub struct PtySession {
    child: Option<Box<dyn Child + Send>>,
    master: Box<dyn MasterPty + Send>,
    killer: Option<PtyKiller>,
    /// 已等待过的退出结果(重复 wait/try_wait 幂等返回)
    exit: Option<crate::process::ExitStatus>,
}

/// PTY 增量读缓冲(阻塞读任务逐块写入;运行中可随时取已读部分)
pub type PtyReadBuffer = std::sync::Arc<std::sync::Mutex<Vec<u8>>>;

/// PTY 终止句柄(与 [`PtySession`] 分离,克隆自由):后台任务的 stop
/// 不持会话也能杀。killer 消费式(首次 kill 后句柄空转);无宽限语义
/// (killer 即杀,平台等价 SIGKILL/TerminateProcess)
#[derive(Clone)]
pub struct PtyKiller {
    killer: std::sync::Arc<std::sync::Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>>,
}

impl PtyKiller {
    /// 以 portable-pty killer 装配(会话与句柄共享同一消费式盒子)
    fn from_boxed(killer: Box<dyn ChildKiller + Send + Sync>) -> Self {
        Self {
            killer: std::sync::Arc::new(std::sync::Mutex::new(Some(killer))),
        }
    }

    /// 立即终止(killer 语义;已消费/已退出则无操作)
    pub fn kill(&self) {
        if let Ok(mut guard) = self.killer.lock()
            && let Some(mut killer) = guard.take()
        {
            killer.kill().ok();
        }
    }
}

/// 在 PTY 中 spawn(program + argv 已按沙箱策略包装)。
pub fn spawn_pty(
    program: &str,
    args: &[String],
    cwd: Option<&std::path::Path>,
    sandbox: Option<&SandboxPolicy>,
) -> Result<PtySession, PtyError> {
    spawn_pty_sized(program, args, cwd, sandbox, PtySize::default(), &[])
}

/// 带初始尺寸与环境注入的 PTY spawn(交互终端用: TERM 等、行列数
/// 需与渲染面一致,否则 TUI 程序按错误网格排版)。其余语义同 [`spawn_pty`]。
pub fn spawn_pty_sized(
    program: &str,
    args: &[String],
    cwd: Option<&std::path::Path>,
    sandbox: Option<&SandboxPolicy>,
    size: PtySize,
    env: &[(&str, &str)],
) -> Result<PtySession, PtyError> {
    // 沙箱 argv 包装(bwrap/seatbelt);landlock-only 返回 Err 即拒绝
    let (program, args) = match sandbox {
        Some(policy) => {
            let probed = crate::sandbox::probe().ok_or_else(|| {
                PtyError::Sandbox(
                    "fail-closed:PTY 执行需要 argv 包装沙箱(bwrap/seatbelt),本机无可用 rung".into(),
                )
            })?;
            if matches!(probed.rung, Rung::Landlock) {
                return Err(PtyError::Sandbox(
                    "fail-closed:PTY 执行需要 argv 包装沙箱(bwrap/seatbelt),本机仅 landlock(pre_exec 不可用于 PTY)".into(),
                ));
            }
            let confined = wrap_argv(policy, program, args, &probed).map_err(|e| {
                PtyError::Sandbox(format!(
                    "fail-closed:PTY 执行需要 argv 包装沙箱(bwrap/seatbelt),\
                     本机不可用({e})"
                ))
            })?;
            (confined.program, confined.argv)
        }
        None => (program.to_string(), args.to_vec()),
    };

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .map_err(|e| PtyError::Io(e.to_string()))?;
    let mut command = CommandBuilder::new(&program);
    command.args(&args);
    for (key, value) in env {
        command.env(key, value);
    }
    if let Some(dir) = cwd {
        command.cwd(dir);
    }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| PtyError::Io(e.to_string()))?;
    let killer = child.clone_killer();
    // slave 必须 drop:子进程持有副本时 master 读不到 EOF
    drop(pair.slave);
    Ok(PtySession {
        child: Some(child),
        master: pair.master,
        killer: Some(PtyKiller::from_boxed(killer)),
        exit: None,
    })
}

impl PtySession {
    /// 起增量读任务:clone reader 阻塞读逐块写入共享缓冲,返回
    /// (缓冲, 任务句柄)。运行中可随时取缓冲的已读部分(超时移交/
    /// 取消拿部分输出);任务收尾即 EOF 读完
    pub fn start_read(&self) -> Result<(PtyReadBuffer, tokio::task::JoinHandle<()>), PtyError> {
        let reader = self
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Io(e.to_string()))?;
        let buf: PtyReadBuffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let target = buf.clone();
        let task = tokio::task::spawn_blocking(move || {
            let mut reader = reader;
            let mut chunk = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut reader, &mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut buf) = target.lock() {
                            buf.extend_from_slice(&chunk[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok((buf, task))
    }

    /// 起通道化增量读:逐块发送到无界通道(EOF/读错关闭发送端;
    /// 消费者掉线即停读)。消费者自行积累与截取部分输出
    pub fn start_read_chunks(
        &self,
    ) -> Result<
        (
            tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
            tokio::task::JoinHandle<()>,
        ),
        PtyError,
    > {
        let reader = self
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Io(e.to_string()))?;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::task::spawn_blocking(move || {
            let mut reader = reader;
            let mut chunk = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut reader, &mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(chunk[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok((rx, task))
    }

    /// 读取全部输出直到 EOF(便捷面 = start_read + await 任务 + 取缓冲;
    /// 取消/超时由调用方在 select 层处理并 kill)
    pub async fn read_to_end(&mut self) -> Result<String, PtyError> {
        let (buf, task) = self.start_read()?;
        task.await.map_err(|e| PtyError::Io(e.to_string()))?;
        let bytes = buf.lock().map(|b| b.clone()).unwrap_or_default();
        Ok(crate::text::decode_output(&bytes))
    }

    /// 分离终止句柄(后台任务 stop 用;与 [`PtySession::kill`] 共享
    /// 同一消费式 killer,先到先杀)
    pub fn killer(&self) -> Option<PtyKiller> {
        self.killer.clone()
    }

    /// 调整 PTY 行列(内核 TIOCSWINSZ,前台进程组收 SIGWINCH)。
    /// 纯 ioctl,任意线程直调;与网格侧 resize 由调用方保持一致
    pub fn resize(&self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(size)
            .map_err(|e| PtyError::Io(e.to_string()))
    }

    /// 取 master 写入口(交互终端的键入/查询应答通道)。每次调用
    /// dup 一个新 fd,会话内不缓存—— [`PtySession::start_write_chunks`]
    /// 消费一次即可;直接拿裸 writer 的调用方自行管理写时机
    pub fn take_writer(&self) -> Result<Box<dyn Write + Send>, PtyError> {
        self.master
            .take_writer()
            .map_err(|e| PtyError::Io(e.to_string()))
    }

    /// 起通道化写任务:与 [`PtySession::start_read_chunks`] 对称,写入
    /// 端 clone 自由、`send` 永不阻塞(PTY 内核缓冲满/`^S` 流控时阻塞
    /// 落在 blocking 池的写任务里,不冻结调用线程)。写失败即收尾;
    /// 发送端全 drop 后 `blocking_recv` 返回 None 自退
    pub fn start_write_chunks(
        &self,
    ) -> Result<
        (
            tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
            tokio::task::JoinHandle<()>,
        ),
        PtyError,
    > {
        let writer = self.take_writer()?;
        let (tx, mut rx): (
            tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
            tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        ) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::task::spawn_blocking(move || {
            let mut writer = writer;
            while let Some(bytes) = rx.blocking_recv() {
                if writer.write_all(&bytes).is_err() || writer.flush().is_err() {
                    break;
                }
            }
        });
        Ok((tx, task))
    }

    /// 非阻塞探测退出(Some = 已退出;None = 还在跑)。探测到即
    /// 缓存,后续 wait/try_wait 幂等返回同一状态
    pub fn try_wait(&mut self) -> Result<Option<crate::process::ExitStatus>, PtyError> {
        if let Some(exit) = self.exit {
            return Ok(Some(exit));
        }
        let Some(child) = self.child.as_mut() else {
            // 句柄已失:仅 wait 消费后可达,彼时 exit 已落定走首分支,
            // 此处不可达的保守值
            return Ok(None);
        };
        let status = child
            .try_wait()
            .map_err(|e| PtyError::Io(e.to_string()))?
            .map(|s| crate::process::ExitStatus {
                // portable-pty 的 ExitStatus 只携带 successful 布尔(见 wait)
                code: if s.success() { Some(0) } else { None },
                signal: None,
            });
        if status.is_some() {
            self.exit = status;
        }
        Ok(status)
    }

    /// 等待子进程退出(幂等;阻塞 wait 经 spawn_blocking)。
    /// 返回完整退出状态(退出码/信号;成功与否由调用方语义决定)
    pub async fn wait(&mut self) -> Result<crate::process::ExitStatus, PtyError> {
        if let Some(exit) = self.exit {
            return Ok(exit);
        }
        let Some(mut child) = self.child.take() else {
            // 句柄已失(重复 wait 后的异常路径):状态不可知,视为失败
            return Ok(crate::process::ExitStatus {
                code: None,
                signal: None,
            });
        };
        // portable-pty 的 ExitStatus 只携带 successful 布尔(无码/信号),
        // 成功即码 0,失败则状态不可知(码/信号留 None)
        let status = tokio::task::spawn_blocking(move || {
            child.wait().map(|s| crate::process::ExitStatus {
                code: if s.success() { Some(0) } else { None },
                signal: None,
            })
        })
        .await
        .map_err(|e| PtyError::Io(e.to_string()))?
        .map_err(|e| PtyError::Io(e.to_string()))?;
        self.exit = Some(status);
        Ok(status)
    }

    /// 杀死子进程(killer 组信号;已退出则无操作)
    pub fn kill(&mut self) {
        if let Some(killer) = &self.killer {
            killer.kill();
        }
    }
}

// 两个用例都是 POSIX 语义(`/bin/bash`、`test -t 1`、`touch`),Windows 侧的
// 对应用例随阶段 4 的 ConPTY 工作落 —— 那时改按平台参数化,而不是各留一份
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    // 用例载荷是 POSIX 语义(`/bin/bash -c`、`test -t 1`、`sandbox-exec`
    // 探测);Windows 侧对应用例随阶段 4 的 ConPTY 工作落
    // (`[Console]::IsOutputRedirected` 断言 tty)。
    #[cfg(unix)]
    #[tokio::test]
    async fn pty_runs_command_with_tty_semantics() {
        // 嵌套沙箱内 openpty 被拒(EPERM):环境性跳过,宿主终端真跑
        if std::process::Command::new("/usr/bin/sandbox-exec")
            .args(["-p", "(version 1)", "true"])
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(false)
        {
            eprintln!("嵌套沙箱内 openpty 不可用:环境性跳过断言");
            return;
        }
        let dir = std::env::temp_dir().join(format!("liuma-pty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut session = spawn_pty(
            "/bin/bash",
            &["-c".into(), "test -t 1 && echo tty-ok".into()],
            Some(&dir),
            None,
        )
        .expect("pty spawn");
        let output = session.read_to_end().await.expect("read");
        let status = session.wait().await.expect("wait");
        assert!(status.success(), "exit 应成功");
        assert!(
            output.contains("tty-ok"),
            "PTY 下 stdout 是 tty;got: {output}"
        );
    }

    // try_wait 与分离句柄:运行中 None → 退出后 Some;killer 不持会话也能杀
    #[cfg(unix)]
    #[tokio::test]
    async fn pty_try_wait_and_detached_killer() {
        if std::process::Command::new("/usr/bin/sandbox-exec")
            .args(["-p", "(version 1)", "true"])
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(false)
        {
            eprintln!("嵌套沙箱内 openpty 不可用:环境性跳过断言");
            return;
        }
        let dir = std::env::temp_dir().join(format!("liuma-pty-tw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut session = spawn_pty(
            "/bin/bash",
            &["-c".into(), "sleep 30".into()],
            Some(&dir),
            None,
        )
        .expect("pty spawn");
        assert!(
            session.try_wait().expect("try_wait").is_none(),
            "运行中应报 None"
        );
        let killer = session.killer().expect("killer");
        killer.kill();
        let status = session.wait().await.expect("wait");
        assert!(!status.success(), "被 killer 终止的进程不应成功退出");
        assert_eq!(
            session.try_wait().expect("try_wait"),
            Some(status),
            "退出后探测返回缓存状态"
        );
    }

    // 载荷是 POSIX 语义(`/bin/bash` + `touch`),Windows 侧对应用例随阶段 4
    // 的 ConPTY 工作落。注:该用例在 Windows 上曾有约 70 秒的额外开销
    // (沙箱 rung 可用后 PTY 路径真跑起来,而 portable-pty 的会话收尾慢),
    // 阶段 4 处理 PTY 时一并查明
    #[cfg(unix)]
    #[tokio::test]
    async fn pty_sandbox_denies_out_of_root_write_on_argv_rungs() {
        use crate::sandbox::SandboxPolicy;
        let dir = std::env::temp_dir().join(format!("liuma-pty-sbx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let outside =
            std::env::temp_dir().join(format!("liuma-pty-outside-{}", std::process::id()));
        // mode 语义下的只读策略:一切写(含 temp 下)均被拒——验证 PTY
        // 路径沙箱生效(argv 包装 rung)
        let policy = SandboxPolicy::read_only();
        let result = spawn_pty(
            "/bin/bash",
            &["-c".into(), format!("touch {}", outside.display())],
            Some(&dir),
            Some(&policy),
        );
        match result {
            Ok(mut session) => {
                let _ = session.read_to_end().await;
                let status = session.wait().await.unwrap_or(crate::process::ExitStatus {
                    code: None,
                    signal: None,
                });
                session.kill();
                assert!(!status.success(), "根外写必须失败(seatbelt/bwrap rung)");
                assert!(!outside.exists());
            }
            Err(PtyError::Sandbox(_)) => {
                // fail-closed 是正确结局,但必须说清是哪种环境:
                // 要么本机无 rung(Windows/探测禁用),要么 rung 不可用于
                // PTY(landlock 无 pre_exec 通道)。**有** argv 包装型 rung
                // 却对 PTY 拒绝执行 = 语义矛盾,不许被这一臂吞掉
                let argv_wrapping = matches!(
                    crate::sandbox::probe().as_ref().map(|p| &p.rung),
                    Some(Rung::Bwrap(_)) | Some(Rung::Seatbelt(_))
                );
                assert!(!argv_wrapping, "本机有 argv 包装 rung,PTY 不该 fail-closed");
            }
            Err(e) => panic!("unexpected: {e}"),
        }
    }

    // 带尺寸 spawn:子进程的 stty 应看到传入的行列(而非 PtySize 默认值)。
    // 载荷是 POSIX 语义(`stty size`),Windows 侧对应用例随阶段 4 落
    #[cfg(unix)]
    #[tokio::test]
    async fn pty_sized_spawn_reports_rows_cols() {
        let dir = std::env::temp_dir().join(format!("liuma-pty-sized-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut session = spawn_pty_sized(
            "/bin/bash",
            &["-c".into(), "stty size".into()],
            Some(&dir),
            None,
            PtySize {
                rows: 33,
                cols: 111,
                pixel_width: 0,
                pixel_height: 0,
            },
            &[],
        )
        .expect("pty spawn");
        let output = session.read_to_end().await.expect("read");
        let status = session.wait().await.expect("wait");
        assert!(status.success(), "exit 应成功");
        assert!(
            output.contains("33 111"),
            "stty 应报初始行列 33 111;got: {output}"
        );
    }

    // 环境注入:CommandBuilder.env 对子进程可见
    #[cfg(unix)]
    #[tokio::test]
    async fn pty_env_injection_reaches_child() {
        let dir = std::env::temp_dir().join(format!("liuma-pty-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut session = spawn_pty_sized(
            "/bin/bash",
            &["-c".into(), "echo $LIUMA_PTY_TEST_VAR".into()],
            Some(&dir),
            None,
            PtySize::default(),
            &[("LIUMA_PTY_TEST_VAR", "env-ok")],
        )
        .expect("pty spawn");
        let output = session.read_to_end().await.expect("read");
        session.wait().await.expect("wait");
        assert!(output.contains("env-ok"), "env 注入应可达;got: {output}");
    }

    // 写通道回路 + resize:writer 写入经 PTY 到达子进程(read 消费),
    // resize 后 stty 应看到新行列。bash 在首个 read 处阻塞等 writer,
    // 以此做读写两端同步;resize 发生在两次 stty 之间。载荷是 POSIX
    // 语义,Windows 侧对应用例随阶段 4 落
    #[cfg(unix)]
    #[tokio::test]
    async fn pty_write_chunks_and_resize() {
        let dir = std::env::temp_dir().join(format!("liuma-pty-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut session = spawn_pty_sized(
            "/bin/bash",
            &[
                "-c".into(),
                "stty size; read line; stty size; echo got-$line".into(),
            ],
            Some(&dir),
            None,
            PtySize {
                rows: 10,
                cols: 40,
                pixel_width: 0,
                pixel_height: 0,
            },
            &[],
        )
        .expect("pty spawn");
        let (writer, _write_task) = session.start_write_chunks().expect("writer");
        let (buf, read_task) = session.start_read().expect("read");
        // 同步点 1:bash 打印初始行列并阻塞在 read
        wait_for(&buf, "10 40").await;
        session
            .resize(PtySize {
                rows: 20,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize");
        // 同步点 2:放行 read,bash 打印 resize 后行列并回显写入内容
        writer.send(b"written-ok\n".to_vec()).expect("send write");
        wait_for(&buf, "got-written-ok").await;
        read_task.await.expect("read task");
        let output = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
        let status = session.wait().await.expect("wait");
        assert!(status.success(), "exit 应成功");
        assert!(
            output.contains("got-written-ok"),
            "写入应经 PTY 到达子进程;got: {output}"
        );
        assert!(
            output.contains("20 80"),
            "resize 后 stty 应报新行列;got: {output}"
        );
    }

    // 轮询共享读缓冲直到出现目标片段(阻塞读任务异步落盘,需让出;
    // 5 秒封顶防悬挂)
    #[cfg(unix)]
    async fn wait_for(buf: &PtyReadBuffer, needle: &str) {
        for _ in 0..100 {
            if let Ok(guard) = buf.lock()
                && String::from_utf8_lossy(&guard).contains(needle)
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("等待 {needle:?} 超时");
    }
}
