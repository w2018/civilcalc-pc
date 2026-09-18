//! 云端备份的临时缓存清理。
//!
//! 源：`civilcalc-android-v2/core/backup/TempCache.kt`（34 行）
//!
//! 打包出来的 `.tar.gz` 与下载下来的备份包都放临时目录。
//! 这些文件可能有几十 MB，**生命周期必须短**。清理策略（由仓库层执行）：
//!
//! | 时机 | 动作 |
//! |---|---|
//! | 打包开始 | **先清空整个目录** —— 兜住上次崩溃/中途退出的残留 |
//! | 上传成功 | 立即删包并再扫一遍目录 |
//! | 上传失败/取消 | **保留**临时包（确认框还在，用户可直接重试，不必重新打包）；关掉确认框即删 |
//! | 下载失败/被取消 | 删掉半包 |
//! | 导入成功或失败 | 都删包并扫目录（失败要重新下载，留着没意义） |

use std::path::Path;

/// 清空目录内容（**不删目录本身**），返回删掉的文件/子项数；目录不存在视为已清空。
///
/// ⚠️ 目录不存在返回 `0` 而不是报错 —— 清理不该因为「本来就没有」而失败。
pub fn clear(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        if delete_recursively(&entry.path()) {
            removed += 1;
        }
    }
    removed
}

/// 删除单个文件。
///
/// `None`、不存在、删除成功**都算成功** —— 清理不该因为「已经不在了」而失败。
#[must_use]
pub fn remove(file: Option<&Path>) -> bool {
    match file {
        None => true,
        Some(p) => !p.exists() || std::fs::remove_file(p).is_ok(),
    }
}

/// 递归删除（目录先删子项再删自身）。
fn delete_recursively(path: &Path) -> bool {
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let _ = delete_recursively(&entry.path());
            }
        }
    }
    std::fs::remove_file(path).is_ok() || std::fs::remove_dir(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("cc-tmpcache-{}", uuid_like()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 不引 uuid（本 crate 没这个依赖）—— 用纳秒时间戳即可
    fn uuid_like() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    #[test]
    fn clear_removes_files_and_counts() {
        let d = tmp_dir();
        std::fs::write(d.join("a.tar.gz"), b"x").unwrap();
        std::fs::write(d.join("b.tar.gz"), b"y").unwrap();
        assert_eq!(clear(&d), 2);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
        assert!(d.exists(), "目录本身不能被删");
    }

    /// 目录不存在 → 返回 0（不是错误）
    #[test]
    fn clear_missing_dir_returns_zero() {
        let missing = std::env::temp_dir().join("cc-tmpcache-definitely-missing-xyz");
        let _ = std::fs::remove_dir_all(&missing);
        assert_eq!(clear(&missing), 0);
    }

    #[test]
    fn clear_empty_dir_returns_zero() {
        let d = tmp_dir();
        assert_eq!(clear(&d), 0);
    }

    /// 子目录被整棵删掉（算 1 个「子项」）
    #[test]
    fn clear_removes_subdirectories_recursively() {
        let d = tmp_dir();
        let sub = d.join("sub");
        std::fs::create_dir_all(sub.join("deep")).unwrap();
        std::fs::write(sub.join("deep").join("f.txt"), b"z").unwrap();
        std::fs::write(d.join("top.txt"), b"t").unwrap();

        assert_eq!(clear(&d), 2, "一个子目录 + 一个文件");
        assert!(!sub.exists());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
    }

    /// 幂等：连清两次第二次为 0
    #[test]
    fn clear_is_idempotent() {
        let d = tmp_dir();
        std::fs::write(d.join("a"), b"x").unwrap();
        assert_eq!(clear(&d), 1);
        assert_eq!(clear(&d), 0);
    }

    // ---------------------------------------------------------------------
    // remove
    // ---------------------------------------------------------------------

    #[test]
    fn remove_none_is_success() {
        assert!(remove(None));
    }

    /// 不存在也算成功（源：`!file.exists() || file.delete()`）
    #[test]
    fn remove_missing_is_success() {
        let p = std::env::temp_dir().join("cc-tmpcache-nope-xyz.tar.gz");
        let _ = std::fs::remove_file(&p);
        assert!(remove(Some(&p)));
    }

    #[test]
    fn remove_existing_file() {
        let d = tmp_dir();
        let f = d.join("x.tar.gz");
        std::fs::write(&f, b"x").unwrap();
        assert!(remove(Some(&f)));
        assert!(!f.exists());
    }

    /// 幂等：删两次都成功
    #[test]
    fn remove_is_idempotent() {
        let d = tmp_dir();
        let f = d.join("y.tar.gz");
        std::fs::write(&f, b"y").unwrap();
        assert!(remove(Some(&f)));
        assert!(remove(Some(&f)));
    }

    /// 传目录进来 → 失败（`remove_file` 删不掉目录）—— 明确区分「删文件」与「清目录」
    #[test]
    fn remove_rejects_directory() {
        let d = tmp_dir();
        assert!(!remove(Some(&d)), "目录要用 clear，不是 remove");
        assert!(d.exists());
    }
}
