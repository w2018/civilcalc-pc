//! Tauri 命令面（14 组，约 90 个命令 —— 见 `docs/05-项目开发方案.md` §1.3.2）。
//!
//! ## 分组与进度
//!
//! | 组 | 域 | 文件 | 任务 | 状态 |
//! |---|---|---|---|---|
//! | 1 | 系统与配置 | `system.rs` | P1-10 | ✅ 7 个 |
//! | 2 | 内置公式库 | `builtin.rs` | P1-10 | ✅ 4 个 |
//! | 3 | 公式 CRUD 与草稿 | `formula.rs` | P1-10 | ✅ 8 个 |
//! | 4 | 求值与校验 | `eval.rs` | P2-1 / P3-6 | ✅ 4 个 |
//! | 5 | 检索 | `search.rs` | P2-4 | ✅ 3 个 |
//! | 6 | 版本 | `version.rs` | P1-12 | ✅ 7 个 |
//! | 7 | 收藏与历史 | `favorite.rs` / `history.rs` | P2-5 | ✅ 7 个 |
//! | 8 | LLM 配置与密钥 | `llm.rs` | P4-7 | ✅ 11 个 |
//! | 9 | AI 生成 | `ai.rs` | P4-6 | ✅ 6 个（流式事件 `ai://*` + 取消 + 用量入库） |
//! | 10 | 模型测试 | `model_test.rs` | P5 | ✅ 5 个（流式事件 `modelTest://*` + 对话持久化 + 自动压缩；复用 `NetState` 取消标志） |
//! | 11 | Token 用量 | `usage.rs` | P4-8 | ✅ 3 个 |
//! | 12 | Excel / 排版 / 验算 | `excel.rs` / `display.rs` / `verify.rs` | P3-4 / P3-6 / P3-8 | ✅ 7 个（Excel 3 + 排版 3 + 验算 1） |
//! | 13 | 报告与导出 | `report.rs` | P3-11 | ✅ 5 个（无 `template_*`，ADR-023） |
//! | 14 | 图片/备份/WebDAV/重置/更新/外观 | `appearance.rs` / `image.rs` / `backup.rs` / `webdav.rs` / `reset.rs` / `update.rs` / `system.rs` | P4-16 ~ P5-3 | 🔶 外观 ✅ 5 / 备份 ✅ 3 / WebDAV ✅ 11 / 图片 ✅ 5 / 重置 ✅ 3 / 更新 ✅ 2 |
//!
//! ## 设计原则
//!
//! - 命令名 `snake_case`；参数与返回字段 `camelCase`
//! - **薄适配**：不写业务逻辑，只做参数整理 + 调业务 crate + 错误转换
//! - 全部返回 `Result<T, CommandError>`；**禁止静默降级**
//! - Rust 侧参数名用 `snake_case`，Tauri 会自动把 **JS 侧的 camelCase 参数名**
//!   映射过来（`formulaId` → `formula_id`），无需额外配置

pub mod ai;
pub mod backup;
pub mod webdav;
pub mod appearance;
pub mod image;
pub mod model_test;
pub mod reset;
pub mod update;
pub mod builtin;
pub mod display;
pub mod eval;
pub mod excel;
pub mod favorite;
pub mod formula;
pub mod history;
pub mod llm;
pub mod report;
pub mod search;
pub mod system;
pub mod usage;
pub mod verify;
pub mod version;

pub use builtin::builtin_formula_count;
