//! PTY spawn:portable-pty 包装 + 沙箱 argv 包装。
//!
//! 用途:需要终端语义的工具(isatty、彩色输出、行缓冲交互)。
//! codex 同款包法(portable-pty + 宿主侧进程管理)。
//!
//! 沙箱:portable-pty 的 CommandBuilder 不暴露 pre_exec,故沙箱只能经
//! argv 包装(bwrap / seatbelt rung)——与 pipes 路径同一探测链。
//! Landlock-only 系统(无法 argv 包装)按 fail-closed **拒绝** PTY 执行,
//! 不静默降级为无沙箱。
//!
//! 读取是阻塞 IO,经 `spawn_blocking` 桥接;master drop 即向会话送
//! SIGHUP(PTY 语义),配合 `kill` 的组信号。

use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
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
        .openpty(PtySize::default())
        .map_err(|e| PtyError::Io(e.to_string()))?;
    let mut command = CommandBuilder::new(&program);
    command.args(&args);
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
}
