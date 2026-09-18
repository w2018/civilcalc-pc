//! # civilcalc-store —— 本地持久化层
//!
//! 源：`civilcalc-android-v2/data/` 模块的持久化部分
//! （`db/CivilCalcDatabase.kt` + `db/dao/*` + `db/entities/*` + `Migrations.kt`）
//!
//! ## 为什么单独一个 crate（ADR-025）
//!
//! `db.rs` 是纯数据访问：只依赖 `rusqlite` 与 `civilcalc-core`，
//! **一行 `tauri` API 都不用**。若把它留在 `src-tauri` 里：
//!
//! - `cargo test` 会连带编译整个 Tauri / WebView 栈（Windows 上很久）
//! - 本模块有 ~35 个单测，是全项目测试最密的地方，迭代会被拖慢
//!
//! 抽成独立 crate 后：`cargo test -p civilcalc-store` 秒级完成，
//! 且"不依赖 tauri"由 `Cargo.toml` 天然强制，不靠自觉。
//!
//! ## 职责边界
//!
//! | 属于本 crate | 不属于本 crate |
//! |---|---|
//! | DDL、迁移、索引 | 业务规则（在 [`civilcalc_core`]） |
//! | 查询 / 写入 / 事务 | 路径解析（在 `src-tauri::paths`） |
//! | 行类型 ↔ SQL 的映射 | 密钥存储（在 `src-tauri::secrets`） |
//! | JSON 列的**原样**存取 | 业务级的 JSON 解析（用 [`civilcalc_core::schema`] 的访问器） |
//!
//! ## 唯一使用 `rusqlite` 的地方
//!
//! 全仓库只有本 crate 依赖 `rusqlite`。命令层、业务 crate 一律通过
//! [`db::Db`] 的方法访问数据，**禁止在别处写 SQL**。
//!
//! ## 备份保真红线（ADR-009）
//!
//! `history` 与 `formula_versions` 里的 JSON 列（`formulaSnapshotJson` /
//! `inputsJson` / `resultJson` / `schemaJson`）**原样读出、原样写回**，
//! 绝不做"解析成结构体再重新序列化" —— 那会改变字段顺序与浮点格式，
//! 破坏备份包的逐字节保真。

pub mod background_store;
pub mod db;
pub mod image_store;
pub mod sha1;

pub use background_store::BackgroundStore;
pub use image_store::{ImageInfo, ImageRecord, LlmImageStore};
pub use db::{
    Db, FavoriteTableRow, UsageDaySummary, UsageRecord, UsageRow, UsageSummary, DB_VERSION,
};
