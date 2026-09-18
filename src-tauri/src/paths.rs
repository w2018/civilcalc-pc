//! 桌面路径解析（替代源项目 `data/paths/AndroidPaths.kt`）。
//!
//! ## 目录布局（见 `docs/04-数据契约.md` §3.3）
//!
//! ```text
//! %APPDATA%\com.w2018.civilcalc.pc\      ← app_data_dir（持久）
//! ├── civilcalc.db (+ -wal / -shm)
//! ├── images/<hash>                       ← LLM 图片（按内容哈希去重）
//! ├── background.jpg                      ← 自定义背景图
//! ├── exports/                            ← 应用私有导出草稿
//! └── backups/                            ← 本地备份
//!
//! %LOCALAPPDATA%\com.w2018.civilcalc.pc\ ← app_cache_dir（可清理）
//! ├── reports/                            ← 预览用临时 docx
//! ├── tmp/                                ← 备份打包临时文件
//! └── logs/                               ← 日志
//! ```
//!
//! ## 清理策略
//!
//! 清理只删 `app_cache_dir`；**绝不删 `app_data_dir` 下的数据库、图片、
//! 背景图与备份**（继承源项目红线）。

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// 桌面路径集合
#[derive(Debug, Clone)]
pub struct DesktopPaths {
    /// 持久数据目录
    pub app_data_dir: PathBuf,
    /// 缓存目录（可清理）
    pub app_cache_dir: PathBuf,
}

impl DesktopPaths {
    /// 解析路径并确保目录存在。
    ///
    /// ## 启动时就创建哪些目录
    ///
    /// | 目录 | 是否启动即建 | 理由 |
    /// |---|---|---|
    /// | `app_data_dir` / `app_cache_dir` | ✅ | 一切的前提 |
    /// | `images/` | ✅ | 图片按内容哈希落盘，第一次用就要求目录存在 |
    /// | `exports/` / `backups/` | ✅ | 用户随时可能导出/备份，提前建好省一次分支 |
    /// | `reports/` / `tmp/` | ✅ | 缓存目录，随手建 |
    /// | `logs/` | ❌ | 由 `tauri-plugin-log` 自己创建（重复创建会与它争抢） |
    pub fn new(app: &AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let app_data_dir = app.path().app_data_dir()?;
        let app_cache_dir = app.path().app_cache_dir()?;

        let me = Self {
            app_data_dir,
            app_cache_dir,
        };

        // 持久目录
        for d in [
            me.app_data_dir.clone(),
            me.images_dir(),
            me.exports_dir(),
            me.backups_dir(),
        ] {
            std::fs::create_dir_all(&d)?;
        }
        // 缓存目录
        for d in [
            me.app_cache_dir.clone(),
            me.reports_dir(),
            me.tmp_dir(),
        ] {
            std::fs::create_dir_all(&d)?;
        }

        Ok(me)
    }

    /// 数据库文件（五张活表）
    pub fn db_path(&self) -> PathBuf {
        self.app_data_dir.join("civilcalc.db")
    }

    /// 应用偏好文件（`AppConfig` 的落盘位置）
    pub fn config_path(&self) -> PathBuf {
        self.app_data_dir.join("config.json")
    }

    /// LLM 图片目录（按内容哈希命名）
    pub fn images_dir(&self) -> PathBuf {
        self.app_data_dir.join("images")
    }

    /// 自定义背景图
    pub fn background_path(&self) -> PathBuf {
        self.app_data_dir.join("background.jpg")
    }

    /// 应用私有导出草稿
    pub fn exports_dir(&self) -> PathBuf {
        self.app_data_dir.join("exports")
    }

    /// 本地备份
    pub fn backups_dir(&self) -> PathBuf {
        self.app_data_dir.join("backups")
    }

    /// 预览用临时 docx
    pub fn reports_dir(&self) -> PathBuf {
        self.app_cache_dir.join("reports")
    }

    /// 备份打包临时文件（流程结束立即清理）
    pub fn tmp_dir(&self) -> PathBuf {
        self.app_cache_dir.join("tmp")
    }

    /// 日志目录。
    ///
    /// ⚠️ **不由本模块创建** —— `tauri-plugin-log` 会自己建；
    /// 这里只提供路径供「打开日志目录」用。
    pub fn logs_dir(&self) -> PathBuf {
        self.app_cache_dir.join("logs")
    }

    /// 测试用构造器：两个目录都指向同一个临时根。
    ///
    /// `DesktopPaths::new` 需要 `AppHandle`，单元测试拿不到 ——
    /// 而命令层里凡是用到路径的逻辑（落盘、同名加序号、越界校验）
    /// 都值得测，所以留这个口子。
    #[cfg(test)]
    pub fn for_test(root: PathBuf) -> Self {
        Self {
            app_data_dir: root.clone(),
            app_cache_dir: root,
        }
    }
}
