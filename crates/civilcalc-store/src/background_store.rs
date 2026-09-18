//! 自定义背景图存取。
//!
//! 源：`data/backup/BackgroundStore.kt`（33 行）
//!
//! ## 路径差异（有意）
//!
//! | | 路径 |
//! |---|---|
//! | 源（Android） | `filesDir/background/bg.jpg` |
//! | PC | `app_data_dir/background.jpg`（P1 已定，见 `paths.rs::background_path`） |
//!
//! 语义完全一致：**整张图一个文件、覆盖写**。所以备份包里它固定是
//! `background.jpg` 这一个条目，跨端互认不受路径影响。
//!
//! ## 🔴 `exists()` 要求「非空」
//!
//! 源：`file().exists() && file().length() > 0`。
//! 0 字节文件按「没设背景图」处理 —— 写到一半断电留下的空文件，
//! 如果算「已设置」，界面会去读它、解码失败，然后显示一个坏掉的背景。

use std::path::{Path, PathBuf};

/// 背景图文件存取
#[derive(Debug, Clone)]
pub struct BackgroundStore {
    file: PathBuf,
}

impl BackgroundStore {
    /// `file` 是**背景图文件本身**（不是目录）
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self { file: file.into() }
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// 是否已设置背景图（**存在且非空**）
    #[must_use]
    pub fn exists(&self) -> bool {
        std::fs::metadata(&self.file).is_ok_and(|m| m.is_file() && m.len() > 0)
    }

    /// 读原始字节；不存在或读失败返回 `None`
    ///
    /// ⚠️ 不区分「不存在」与「读失败」——调用方对两者的处理相同（当作没有背景图）。
    #[must_use]
    pub fn read(&self) -> Option<Vec<u8>> {
        std::fs::read(&self.file).ok()
    }

    /// 覆盖写入（自动建父目录）
    pub fn write(&self, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(parent) = self.file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.file, bytes)
    }

    /// 删除背景图；返回**是否真的删了一个**（没设过背景图时返回 `false`）
    ///
    /// 源同：返回值让「重置」功能能准确报告「有没有东西被清掉」。
    pub fn clear(&self) -> bool {
        if !self.file.exists() {
            return false;
        }
        std::fs::remove_file(&self.file).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "civilcalc-bg-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn missing_file_is_not_set() {
        let dir = tempdir("missing");
        let s = BackgroundStore::new(dir.join("background.jpg"));
        assert!(!s.exists());
        assert!(s.read().is_none());
        assert!(!s.clear(), "没东西可删时返回 false");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 🔴 0 字节文件按「没设」处理（写到一半断电的残留）
    #[test]
    fn empty_file_is_not_set() {
        let dir = tempdir("empty");
        let path = dir.join("background.jpg");
        std::fs::write(&path, b"").unwrap();
        let s = BackgroundStore::new(&path);
        assert!(!s.exists(), "0 字节不算已设置");
        // 但文件确实在，clear 仍能删掉它
        assert!(s.clear());
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_then_read_roundtrip() {
        let dir = tempdir("roundtrip");
        let s = BackgroundStore::new(dir.join("background.jpg"));
        let bytes = vec![0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

        s.write(&bytes).unwrap();
        assert!(s.exists());
        assert_eq!(s.read().unwrap(), bytes);

        // 覆盖写（不留旧字节）
        s.write(b"short").unwrap();
        assert_eq!(s.read().unwrap(), b"short");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 父目录不存在时自动创建（首次设置背景图）
    #[test]
    fn write_creates_parent_dir() {
        let dir = tempdir("mkdir");
        let s = BackgroundStore::new(dir.join("deep").join("nested").join("bg.jpg"));
        s.write(b"x").unwrap();
        assert!(s.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clear_reports_whether_something_was_removed() {
        let dir = tempdir("clear");
        let s = BackgroundStore::new(dir.join("background.jpg"));
        s.write(b"x").unwrap();
        assert!(s.clear(), "删掉了 → true");
        assert!(!s.exists());
        assert!(!s.clear(), "再删一次 → false");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `file()` 暴露真实路径（调用方据此落盘/展示）
    #[test]
    fn file_is_the_exact_path() {
        let dir = tempdir("path");
        let path = dir.join("background.jpg");
        let s = BackgroundStore::new(path.clone());
        assert_eq!(s.file(), path.as_path());
        std::fs::remove_dir_all(&dir).ok();
    }
}
