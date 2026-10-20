//! `~/.pi-flash/projects.json` — PF 项目注册表（004 打开项目菜单 / psp 分组）。
//!
//! 背景：项目列表原本是「30 天内有会话活动的 cwd 聚合」——pi 会话文件的
//! 投影。只开项目/文件不对话则磁盘零痕迹，项目在两个面板都不出现。
//! 定案（用户 2026-10-16）：**打开项目即注册**。
//!
//! - `register`：switch_project / 启动恢复落进的项目 upsert + touch now
//! - 列表层（app 侧）取「注册表 ∪ 存量会话扫描」并集：注册表是增量，
//!   既有项目仍来自 pi 会话扫描，两处按 [`path_key`]（同 app 侧 ws_key
//!   口径）去重合并
//! - 打开后的项目即使零会话也活跃：同时出现在 psp 项目分组与打开项目
//!   弹窗；30 天窗口同样适用于注册表时刻（长期不开自然淡出）
//! - 「删除所有会话移除项目」走 [`Registry::remove`] 同步清册
//!
//! 结构仿 recents.rs：可测内核（显式 file 参数）+ 进程级单例薄壳。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{read_json, write_json};

/// 注册表容量：按最近打开排序的硬上限（防御性；真实项目数远小于此）。
pub const PROJECTS_CAPACITY: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub path: PathBuf,
    /// 最近一次「打开项目」时刻（ms since epoch）。
    pub last_opened_ms: i64,
}

/// 归一化项目键：与 app 侧 `services::workspace::ws_key` 同口径。
///
/// 口径按平台分：
/// - Windows：`/`→`\`、去尾分隔符、**大小写折叠**（NTFS 大小写不敏感）；
/// - macOS/Linux：只去尾分隔符，**保留大小写**、保留 `/`（APFS 默认
///   大小写敏感，`~/Proj` 与 `~/proj` 是两个目录；把 `/` 折成 `\` 还会
///   让 mac 上所有路径看起来像同一个 Windows 路径）。
pub fn path_key(path: &Path) -> String {
    #[cfg(windows)]
    {
        let mut k = path.to_string_lossy().replace('/', "\\");
        while k.ends_with('\\') {
            k.pop();
        }
        k.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        let mut k = path.to_string_lossy().into_owned();
        // 根目录 "/" 去掉尾分隔符会变空串，退化成「所有路径都匹配」——保留。
        while k.len() > 1 && k.ends_with('/') {
            k.pop();
        }
        k
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 可测内核：entries 恒按 last_opened_ms 降序、按 [`path_key`] 去重。
pub struct Registry {
    file: PathBuf,
    entries: Vec<ProjectRecord>,
}

impl Registry {
    pub fn load(file: &Path) -> Registry {
        let mut entries: Vec<ProjectRecord> = read_json(file)
            .ok()
            .and_then(|v| serde_json::from_value::<Vec<ProjectRecord>>(v["projects"].clone()).ok())
            .unwrap_or_default();
        entries.sort_by(|a, b| b.last_opened_ms.cmp(&a.last_opened_ms));
        let mut seen = std::collections::HashSet::new();
        entries.retain(|e| seen.insert(path_key(&e.path)));
        entries.truncate(PROJECTS_CAPACITY);
        Registry {
            file: file.to_path_buf(),
            entries,
        }
    }

    fn save(&self) {
        let value = serde_json::json!({ "version": 1, "projects": self.entries });
        // 自有目录可能还没建（首启）——写前确保存在
        let _ = crate::paths::ensure_dir();
        let _ = write_json(&self.file, &value);
    }

    /// upsert + touch：打开时刻恒取给定值（调用方传 now，恒新于旧值）；
    /// 同项目多拼写（盘符大小写/正反斜杠）合并，最新拼写上屏（同 recents
    /// promote 的 freshest-wins 口径）。
    pub fn register_at(&mut self, path: &Path, opened_ms: i64) {
        let key = path_key(path);
        self.entries.retain(|e| path_key(&e.path) != key);
        let at = self
            .entries
            .partition_point(|e| e.last_opened_ms > opened_ms);
        self.entries.insert(
            at,
            ProjectRecord {
                path: path.to_path_buf(),
                last_opened_ms: opened_ms,
            },
        );
        self.entries.truncate(PROJECTS_CAPACITY);
        self.save();
    }

    /// 最近打开降序全量（列表层聚合用）。
    pub fn records(&self) -> &[ProjectRecord] {
        &self.entries
    }

    /// 项目被删除（psp「删除所有会话移除项目」）时同步清册。
    pub fn remove(&mut self, path: &Path) {
        let key = path_key(path);
        let before = self.entries.len();
        self.entries.retain(|e| path_key(&e.path) != key);
        if self.entries.len() != before {
            self.save();
        }
    }

    /// 清洗应用自有路径条目（install_root 显式注入便于测试；None = 无从
    /// 判定，原样返回 0）。有清除才落盘，返回清除条数。
    pub fn purge_app_paths_at(&mut self, install_root: Option<&Path>) -> usize {
        let Some(root) = install_root else { return 0 };
        let before = self.entries.len();
        self.entries
            .retain(|e| !crate::paths::is_app_install_path_at(&e.path, root));
        let removed = before - self.entries.len();
        if removed > 0 {
            self.save();
        }
        removed
    }
}

// ---------------------------------------------------------------------------
// 进程级单例薄壳（app 侧只碰这三个函数）
// ---------------------------------------------------------------------------

static REGISTRY: std::sync::Mutex<Option<Registry>> = std::sync::Mutex::new(None);

fn with_registry<T>(f: impl FnOnce(&mut Registry) -> T) -> Option<T> {
    let mut guard = REGISTRY.lock().ok()?;
    let reg = guard.get_or_insert_with(|| {
        Registry::load(
            &crate::paths::projects_file()
                .unwrap_or_else(|| PathBuf::from("pi-flash-projects-fallback.json")),
        )
    });
    Some(f(reg))
}

/// 打开项目即注册：upsert + touch now + 落盘。
pub fn register_project(path: &Path) {
    // 应用自有目录（载荷目录 / npm 包根）不是用户项目，永不入册：0.1.x
    // 更新交接 spawn 不设 cwd，新进程继承旧载荷目录曾被当项目写进来
    //（存量由 `purge_app_paths` 在 boot 清洗）
    if crate::paths::is_app_install_path(path) {
        return;
    }
    let ms = now_ms();
    let _ = with_registry(|reg| reg.register_at(path, ms));
}

/// 启动清洗（幂等）：注册表里落在当前进程安装根内的存量——历史交接写脏
/// 的「payload」等项目——一次性清掉并落盘。
pub fn purge_app_paths() -> usize {
    with_registry(|reg| reg.purge_app_paths_at(crate::paths::current_install_root().as_deref()))
        .unwrap_or(0)
}

/// 最近打开降序的项目记录快照。
pub fn project_records() -> Vec<ProjectRecord> {
    with_registry(|reg| reg.records().to_vec()).unwrap_or_default()
}

/// 项目删除时同步清册。
pub fn remove_project(path: &Path) {
    let _ = with_registry(|reg| reg.remove(path));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pi-link-projects-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("projects.json")
    }

    #[test]
    #[cfg(windows)]
    fn register_upserts_and_touches() {
        let mut r = Registry::load(&temp_file("upsert"));
        r.register_at(Path::new("D:\\a\\demo"), 1_000);
        r.register_at(Path::new("D:\\b\\older"), 500);
        assert_eq!(r.records()[0].path, PathBuf::from("D:\\a\\demo"));
        // re-register bumps to the top, freshest spelling wins (recents 口径)
        r.register_at(Path::new("D:/B/OLDER"), 2_000);
        assert_eq!(r.records()[0].path, PathBuf::from("D:/B/OLDER"));
        assert_eq!(r.records().len(), 2, "normalized key merges spellings");
    }

    #[test]
    #[cfg(not(windows))]
    fn register_upserts_and_touches() {
        let mut r = Registry::load(&temp_file("upsert"));
        r.register_at(Path::new("/a/demo"), 1_000);
        r.register_at(Path::new("/b/older"), 500);
        assert_eq!(r.records()[0].path, PathBuf::from("/a/demo"));
        // re-register bumps to the top（尾分隔符仍折叠）
        r.register_at(Path::new("/a/demo/"), 2_000);
        assert_eq!(r.records()[0].path, PathBuf::from("/a/demo/"));
        assert_eq!(r.records().len(), 2, "trailing separator still merges");
    }

    #[test]
    #[cfg(windows)]
    fn path_key_normalizes_like_ws_key() {
        assert_eq!(
            path_key(Path::new("d:/a\\B/")),
            path_key(Path::new("D:\\A\\b"))
        );
        assert_eq!(path_key(Path::new("D:\\x")), "d:\\x");
    }

    #[test]
    #[cfg(not(windows))]
    fn path_key_normalizes_like_ws_key() {
        // 尾分隔符折叠，不折叠大小写
        assert_eq!(
            path_key(Path::new("/a/b/")),
            path_key(Path::new("/a/b"))
        );
        assert_eq!(path_key(Path::new("/x")), "/x");
        // POSIX 大小写敏感：两个目录必须不同键
        assert_ne!(path_key(Path::new("/Proj")), path_key(Path::new("/proj")));
        // 根目录不能折成空串
        assert_eq!(path_key(Path::new("/")), "/");
    }

    #[test]
    #[cfg(not(windows))]
    fn remove_drops_entry() {
        let mut r = Registry::load(&temp_file("rm"));
        r.register_at(Path::new("/a"), 10);
        r.register_at(Path::new("/b"), 20);
        r.remove(Path::new("/a/"));
        assert_eq!(r.records().len(), 1);
        assert_eq!(r.records()[0].path, PathBuf::from("/b"));
    }

    #[test]
    #[cfg(windows)]
    fn remove_drops_entry() {
        let mut r = Registry::load(&temp_file("rm"));
        r.register_at(Path::new("D:\\a"), 10);
        r.register_at(Path::new("D:\\b"), 20);
        r.remove(Path::new("d:/A"));
        assert_eq!(r.records().len(), 1);
        assert_eq!(r.records()[0].path, PathBuf::from("D:\\b"));
    }

    #[test]
    fn capacity_truncates_oldest() {
        let mut r = Registry::load(&temp_file("cap"));
        for i in 0..(PROJECTS_CAPACITY + 10) {
            r.register_at(
                Path::new(&format!("D:\\p{i}")),
                (PROJECTS_CAPACITY + 20 - i) as i64,
            );
        }
        assert_eq!(r.records().len(), PROJECTS_CAPACITY);
        assert_eq!(r.records()[0].path, PathBuf::from("D:\\p0"));
    }

    #[test]
    #[cfg(windows)]
    fn purge_app_paths_drops_install_dir_entries() {
        let mut r = Registry::load(&temp_file("purge"));
        r.register_at(Path::new("D:\\softwares\\pi-flash\\payload"), 3_000);
        r.register_at(Path::new("D:\\work"), 2_000);
        // None = 无从判定，不动
        assert_eq!(r.purge_app_paths_at(None), 0);
        assert_eq!(r.records().len(), 2);
        let removed = r.purge_app_paths_at(Some(Path::new("D:/SOFTWARES/pi-flash")));
        assert_eq!(removed, 1, "载荷目录条目清除，用户项目保留");
        assert_eq!(r.records().len(), 1);
        assert_eq!(r.records()[0].path, PathBuf::from("D:\\work"));
        // 幂等
        assert_eq!(
            r.purge_app_paths_at(Some(Path::new("D:/SOFTWARES/pi-flash"))),
            0
        );
    }

    /// POSIX 版：安装根判定按 `/` 分隔、大小写敏感。
    #[test]
    #[cfg(not(windows))]
    fn purge_app_paths_drops_install_dir_entries() {
        let mut r = Registry::load(&temp_file("purge"));
        r.register_at(Path::new("/opt/pi-flash/payload"), 3_000);
        r.register_at(Path::new("/work"), 2_000);
        assert_eq!(r.purge_app_paths_at(None), 0);
        assert_eq!(r.records().len(), 2);
        let removed = r.purge_app_paths_at(Some(Path::new("/opt/pi-flash")));
        assert_eq!(removed, 1, "载荷目录条目清除，用户项目保留");
        assert_eq!(r.records().len(), 1);
        assert_eq!(r.records()[0].path, PathBuf::from("/work"));
        assert_eq!(r.purge_app_paths_at(Some(Path::new("/opt/pi-flash"))), 0);
    }

    #[test]
    fn persists_across_load() {
        let file = temp_file("persist");
        {
            let mut r = Registry::load(&file);
            r.register_at(Path::new("D:\\a\\demo"), 7);
        }
        let r = Registry::load(&file);
        assert_eq!(
            r.records(),
            &[ProjectRecord {
                path: PathBuf::from("D:\\a\\demo"),
                last_opened_ms: 7,
            }]
        );
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn corrupt_file_loads_empty() {
        let file = temp_file("corrupt");
        std::fs::write(&file, b"{ not json").unwrap();
        let r = Registry::load(&file);
        assert!(r.records().is_empty());
        let _ = std::fs::remove_file(&file);
    }
}
