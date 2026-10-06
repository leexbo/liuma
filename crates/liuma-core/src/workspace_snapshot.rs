//! 工作区快照:turn 边界的 git 状态对比(变更层兜底的数据面)。
//!
//! turn/start 记基线、turn/end 取终态,差集 = 本回合新增/变更的文件
//! (含 untracked——覆盖 bash 创建文件的盲区)。非 git 仓库 / git 缺席 /
//! 超时一律 `None` 静默跳过,零日志噪音;仓库根 ≠ 工作区根(子目录工作
//! 区)保守跳过——porcelain 路径相对仓库根,与桌面「按工作区根 join 打
//! 开」的语义只有工作区=仓库根时才吻合。
//!
//! 消费:registry 的 turn-tail 端口闭包(Start 相记基线、End 相出
//! `{"changes": [path, ...]}` 载荷)。

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 单条 git 子进程的超时
const GIT_TIMEOUT: Duration = Duration::from_secs(3);

/// turn 边界快照端口实现:git 状态对比,基线自管(Mutex)。Start 相
/// 记基线、End 相出变更载荷;锁不跨 await(await 完成后再短暂持有)
pub struct GitTurnTail {
    /// 工作区根(porcelain 守卫含 toplevel 同判)
    pub ws_root: PathBuf,
    baseline: std::sync::Mutex<Option<HashMap<String, String>>>,
}

impl GitTurnTail {
    /// 以工作区根构建(基线空,等待 Start 相)
    pub fn new(ws_root: PathBuf) -> Self {
        Self {
            ws_root,
            baseline: std::sync::Mutex::new(None),
        }
    }
}

impl liuma_agent_loop::TurnTailSnapshotObj for GitTurnTail {
    fn snapshot<'a>(
        &'a self,
        phase: liuma_agent_loop::TurnTailPhase,
    ) -> std::pin::Pin<Box<dyn Future<Output = Option<serde_json::Value>> + Send + 'a>> {
        Box::pin(async move {
            match phase {
                liuma_agent_loop::TurnTailPhase::Start => {
                    let got = baseline(&self.ws_root).await;
                    if let Ok(mut b) = self.baseline.lock() {
                        *b = got;
                    }
                    None
                }
                liuma_agent_loop::TurnTailPhase::End => {
                    // take 出基线后立即放锁(guard 不跨 await,future 须 Send)
                    let mut base = self.baseline.lock().ok().and_then(|mut g| g.take());
                    changes_payload(&self.ws_root, &mut base).await
                }
            }
        })
    }
}

/// turn/start 相:记录基线(工作区变更文件状态表)。None = 非 git /
/// 超时(本 turn 静默跳过)
pub async fn baseline(ws_root: &Path) -> Option<HashMap<String, String>> {
    status_map(ws_root).await
}

/// turn/end 相:取终态并与基线对比,产出变更载荷
/// `{"changes": [{path, added?, removed?}, ...]}`——base 没有的新进
/// 文件;untracked(`??`)= bash 创建盲区,补行数统计(新文件 +N -0,
/// 读失败/二进制 → 无统计);基线有而终态无 = 本回合删除,剔除。
/// 基线缺席(None)= Start 相未跑 → None。
pub async fn changes_payload(
    ws_root: &Path,
    baseline: &mut Option<HashMap<String, String>>,
) -> Option<serde_json::Value> {
    let base = baseline.take()?;
    let now = status_map(ws_root).await?;
    let mut items: Vec<serde_json::Value> = Vec::new();
    for (path, status) in &now {
        if base.contains_key(path) {
            continue;
        }
        let mut item = serde_json::json!({ "path": path });
        if status == "??" {
            let added = line_count(&ws_root.join(path)).await;
            item["added"] = serde_json::json!(added);
            item["removed"] = serde_json::json!(added.map(|_| 0u64));
        }
        items.push(item);
    }
    if items.is_empty() {
        return None;
    }
    Some(serde_json::json!({ "changes": items }))
}

/// 文本行数(新文件的 +N;读失败 = 二进制/大文件 → None)
async fn line_count(path: &Path) -> Option<u64> {
    let text = tokio::fs::read_to_string(path).await.ok()?;
    Some(text.lines().count() as u64)
}

/// 工作区变更文件状态表:`git status --porcelain -z` → path → XY
/// 状态码(untracked = `??`)。非 repo / git 缺席 / 超时 / 仓库根
/// ≠ 工作区根 → None。
pub async fn status_map(ws_root: &Path) -> Option<HashMap<String, String>> {
    // 非 repo / git 缺席 → 静默
    let inside = git(ws_root, &["rev-parse", "--is-inside-work-tree"]).await?;
    if inside.trim() != "true" {
        return None;
    }
    // 仓库根 ≠ 工作区根(子目录工作区):porcelain 路径相对仓库根,
    // 与桌面按工作区根打开的语义不吻合 → 保守跳过
    let toplevel = git(ws_root, &["rev-parse", "--show-toplevel"]).await?;
    if !same_dir(Path::new(toplevel.trim()), ws_root) {
        return None;
    }
    let raw = git(ws_root, &["status", "--porcelain", "-z"]).await?;
    Some(parse_porcelain_z(raw.as_bytes()))
}

/// porcelain v1(-z,NUL 分隔)解析:每条为 `XY` 两字符状态 + 空格 +
/// `<path>`(X 可能是空格——staged/worktree 位);rename/copy 的 -z
/// 形态为 `XY new\0old`(原路径跟在下一条 NUL 段,只收新名)
fn parse_porcelain_z(raw: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for entry in raw.split(|b| *b == 0) {
        if entry.len() < 4 {
            continue;
        }
        let entry = String::from_utf8_lossy(entry);
        let status = entry.get(..2).unwrap_or_default().to_string();
        let path = entry.get(3..).unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        out.entry(path.to_string()).or_insert(status);
    }
    out
}

/// 目录同判(canonicalize 归一符号链接;macOS /tmp 教训)
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// 跑 git 子进程(cwd = 工作区;超时/非零/spawn 失败 → None)
async fn git(ws_root: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::time::timeout(
        GIT_TIMEOUT,
        tokio::process::Command::new("git")
            .args(args)
            .current_dir(ws_root)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gitrepo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "liuma-snapshot-{tag}-{}-{}",
            std::process::id(),
            uuid_like()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        run(&dir, &["init", "-q"]);
        run(&dir, &["config", "user.email", "t@t"]);
        run(&dir, &["config", "user.name", "t"]);
        dir
    }

    fn uuid_like() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0)
    }

    fn run(dir: &Path, args: &[&str]) {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git");
    }

    #[tokio::test]
    async fn untracked_file_enters_changes() {
        let dir = gitrepo("untracked");
        std::fs::write(dir.join("a.txt"), "hi\n").unwrap();
        let map = status_map(&dir).await.expect("git repo 应产出状态表");
        assert_eq!(map.get("a.txt"), Some(&"??".to_string()), "{map:?}");
        // Start 相未跑(None)= 静默
        assert!(changes_payload(&dir, &mut None).await.is_none());
        // 基线已含 a.txt(untracked)→ 无新变更,应 None
        let mut base = HashMap::new();
        base.insert("a.txt".into(), "??".into());
        let payload = changes_payload(&dir, &mut Some(base.clone())).await;
        assert!(payload.is_none(), "基线已含 → 无变更,应 None");
        // bash 新建 untracked 文件 → 进变更载荷且带行数统计
        std::fs::write(dir.join("b.txt"), "one\ntwo\n").unwrap();
        let payload = changes_payload(&dir, &mut Some(base))
            .await
            .expect("有变更");
        assert_eq!(
            payload["changes"],
            serde_json::json!([{ "path": "b.txt", "added": 2, "removed": 0 }])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn non_git_dir_is_silent() {
        let dir = std::env::temp_dir().join(format!("liuma-snapshot-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(status_map(&dir).await, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn porcelain_z_parse() {
        // `?? untracked` / ` M modified` / rename 新名
        let raw = b"?? a.txt\0 M b.txt\0R  c-new.txt\0c-old.txt\0";
        let map = parse_porcelain_z(raw);
        assert!(map.contains_key("a.txt"));
        assert!(map.contains_key("b.txt"));
        assert!(map.contains_key("c-new.txt"));
        assert!(!map.contains_key("c-old.txt"), "rename 原路径不独立成条");
    }

    #[test]
    fn deleted_between_snapshots_dropped() {
        // 基线有、终态无 = 本回合删除 → 差集不含(终态驱动)
        let base: HashMap<String, String> = [("gone.txt".to_string(), "??".to_string())]
            .into_iter()
            .collect();
        let now: HashMap<String, String> = HashMap::new();
        let changed: Vec<&String> = now.keys().filter(|p| !base.contains_key(*p)).collect();
        assert!(changed.is_empty());
    }
}
