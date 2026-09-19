//! # civilcalc-pc —— AI全能计算器 PC 端应用壳
//!
//! ## 职责（见 `docs/05-项目开发方案.md` §1.2.3）
//!
//! 参数整理、状态注入、调用业务 crate、错误转换、平台能力
//! （文件对话框 / keyring / 落盘）。**不写业务规则**。
//!
//! ## 硬约束
//!
//! - 本 crate 是**唯一依赖 `tauri`** 的地方
//! - 五个业务 crate（core / store / llm / backup / report）不得依赖 tauri
//!   （ADR-002 / ADR-025）
//! - **不直接写 SQL**：数据访问统一走 [`civilcalc_store::Db`]（ADR-025）

pub mod commands;
pub mod config;
pub mod error;
pub mod export;
pub mod paths;
pub mod secrets;
pub mod startup;
pub mod state;

use serde::{Deserialize, Serialize};

/// 应用信息（"关于"页 + 排障用）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// 应用版本（来自 `Cargo.toml`）
    pub version: String,
    /// 产品名（来自 `tauri.conf.json`）
    pub product_name: String,
    /// 应用标识
    pub identifier: String,
    /// 数据库路径
    pub db_path: String,
    /// 数据目录（持久）
    pub app_data_dir: String,
    /// 缓存目录（可清理）
    pub app_cache_dir: String,
    /// 日志目录（由 `tauri-plugin-log` 创建）
    pub logs_dir: String,
    /// 内置公式条数（编译进二进制的 JSON 条数，**不含**用户公式）
    pub builtin_formula_count: usize,
    /// 库中公式总行数（内置播种 + 用户自建）
    pub formula_count: usize,
    /// 数据库 `user_version`（排障关键信息）
    pub db_schema_version: i32,
}

/// 启动 Tauri 应用。
pub fn run() {
    tauri::Builder::default()
        // 单实例：重复启动时聚焦已有主窗口，不再另开进程
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        // ============================================================
        // 🔴 日志插件的配置**只能写在这里**，不能写进 `tauri.conf.json`
        //
        // `tauri-plugin-log` 内部用的是 `plugin::Builder::new("log")`，
        // 它的配置类型是默认的 `()` —— **不从配置文件读参数**。
        // 一旦在 `tauri.conf.json` 里写了 `plugins.log = { … }`，
        // Tauri 会拿这个 map 去反序列化成 `()`，直接 panic：
        //
        //   PluginInitialization("log", "Error deserializing 'plugins.log'
        //   within your Tauri configuration: invalid type: map, expected unit")
        //
        // release 构建是 `windows_subsystem = "windows"`（无控制台），
        // 所以这个 panic 的表现是**双击图标完全没反应**，极难排查。
        //
        // 原配置意图：targets = stdout + logDir，level = info。
        // ⚠️ `LogDir` 在 Windows 上是 `%LOCALAPPDATA%\{identifier}\logs`
        //    （**Local**AppData，不是 Roaming —— 数据库在 Roaming）。
        // ============================================================
        .plugin(
            tauri_plugin_log::Builder::default()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            // ============================================================
            // 启动时序（对齐源项目 CivilCalcApp.onCreate）
            //
            // 源项目顺序：
            //   1. CivilLog 桥接 Logcat
            //   2. CivilCalcDatabase.getInstance()  ← 建库 + 迁移（1→2→3）
            //   3. 读 assets/builtin_formulas.json → 逐条校验 → 播种
            //   4. Excel 字段迁移（schemaVersion < 3）
            //
            // PC 端顺序（见 docs/07-实施路线图与任务清单.md P1-6）：
            //   1. DesktopPaths 解析（app_data_dir / app_cache_dir）
            //   2. Db::open()  ← 建目录 + 建表 + user_version 迁移
            //   3. startup::init_store()  内置库校验 → 播种
            //      ⚠️ 校验失败必须拒绝启动（源项目行为）
            //   4. TODO(P3-1) excel::migrator 迁移 schemaVersion < 3 的公式
            //   5. TODO(P2-4) search::index 重建
            // ============================================================

            // 1. 路径
            let paths = paths::DesktopPaths::new(app.handle())
                .map_err(|e| format!("解析应用目录失败: {e}"))?;

            // 2. 数据库（建目录 + 建表 + user_version 迁移）
            let db = civilcalc_store::Db::open(&paths.db_path())
                .map_err(|e| format!("初始化数据库失败: {e}"))?;

            // 3. 内置库校验 + 首次播种
            //    ⚠️ 这里用 `?` 直接把错误抛给 Tauri：校验失败 → setup 失败 → 应用不启动
            let (report, index) = startup::init_store(&db)?;

            // 4. 应用偏好
            //    文件损坏时**降级为默认值**，不拒绝启动 —— 偏好丢了不该让用户打不开应用。
            //    坏文件改名留证（不删），便于事后排查。
            let config_path = paths.config_path();
            let config = match config::AppConfig::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    let broken = config_path.with_extension("json.broken");
                    let note = match std::fs::rename(&config_path, &broken) {
                        Ok(()) => format!("已备份为 {}", broken.display()),
                        Err(re) => format!("备份失败（{re}），原文件保留"),
                    };
                    civilcalc_core::log::w(
                        "Startup",
                        &format!("偏好文件读取失败，{note}，本次使用默认值: {e}"),
                        None,
                    );
                    // 原文件已被移走 → 这里得到全默认值
                    config::AppConfig::load(&config_path)
                        .unwrap_or_else(|_| config::AppConfig::in_memory())
                }
            };

            // 5. TODO(P3-1): Excel 字段迁移
            // 6. TODO(P2-4): 搜索索引重建

            civilcalc_core::log::i(
                "Startup",
                &format!(
                    "启动完成：内置库 {} 条（跳过 {}），本次播种 {} 条，偏好 {} 项，数据库 {}",
                    report.builtin_valid,
                    report.builtin_skipped.len(),
                    report.seeded,
                    config.keys().len(),
                    paths.db_path().display()
                ),
            );

            // `app.manage` 来自 `tauri::Manager` trait，须显式引入
            use tauri::Manager;
            app.manage(state::AppState::new(paths, db, config, index));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // ---- 组 1：系统与配置 ----
            commands::system::app_info,
            commands::system::init_db,
            commands::system::config_get,
            commands::system::config_save,
            commands::system::config_reset_section,
            commands::system::compile_cache_stats,
            commands::system::compile_cache_clear,
            commands::system::eula_status,
            commands::system::eula_accept,
            commands::system::app_exit,
            // ---- 组 2：内置公式库 ----
            commands::builtin::builtin_formulas,
            commands::builtin::builtin_formulas_json,
            commands::builtin::builtin_status,
            commands::builtin::builtin_reload,
            // ---- 组 3：公式 CRUD 与草稿 ----
            commands::formula::formula_get,
            commands::formula::formula_list,
            commands::formula::formula_save,
            commands::formula::formula_delete,
            commands::formula::formula_exists,
            commands::formula::formula_draft_save,
            commands::formula::formula_draft_load,
            commands::formula::formula_draft_clear_others,
            // ---- 组 6：版本 ----
            commands::version::version_list,
            commands::version::version_get,
            commands::version::version_head,
            commands::version::version_switch,
            commands::version::version_create,
            commands::version::version_diff,
            commands::version::version_resolve_head,
            // ---- 组 7：收藏与历史 ----
            commands::favorite::is_favorite,
            commands::favorite::toggle_favorite,
            commands::favorite::favorites_list,
            commands::history::history_list,
            commands::history::history_get,
            commands::history::history_record_created,
            commands::history::history_delete,
            commands::history::history_clear,
            commands::history::history_thinking,
            // ---- 组 8：LLM 配置与密钥 ----
            commands::llm::llm_list_profiles,
            commands::llm::llm_save_profile,
            commands::llm::llm_update_profile,
            commands::llm::llm_set_active,
            commands::llm::llm_delete_profile,
            commands::llm::llm_set_api_key,
            commands::llm::llm_has_api_key,
            commands::llm::llm_delete_api_key,
            commands::llm::llm_test_connection,
            commands::llm::llm_config_export,
            commands::llm::llm_config_import,
            // ---- 组 11：Token 用量 ----
            commands::usage::usage_stats,
            commands::usage::usage_clear,
            commands::usage::usage_export_csv,
            // ---- 组 14a：外观与背景 ----
            commands::appearance::appearance_get,
            commands::appearance::appearance_save,
            commands::appearance::background_get,
            commands::appearance::background_save,
            commands::appearance::background_clear,
            commands::appearance::background_default_transparency,
            // ---- 组 4：求值与校验 ----
            commands::eval::eval_schema,
            commands::eval::eval_formula,
            commands::eval::validate_schema_cmd,
            commands::eval::eval_steps,
            // ---- 组 5：检索 ----
            commands::search::search_formulas,
            commands::search::search_suggest,
            commands::search::search_rebuild_index,
            // ---- 组 12a：Excel 转换 / 校验 / 迁移 ----
            commands::excel::excel_convert,
            commands::excel::excel_validate,
            commands::excel::excel_migrate,
            // ---- 组 12b：二维数学排版 ----
            commands::display::math_layout,
            commands::display::math_layout_equation,
            commands::display::math_layout_result_line,
            // ---- 组 12c：方程代入验算 ----
            commands::verify::verify_equations,
            // ---- 组 13：报告与导出 ----
            commands::report::export_options_get,
            commands::report::export_options_save,
            commands::report::report_preview,
            commands::report::report_export_docx,
            commands::report::report_reveal,
            // ---- 组 9：AI 生成 ----
            commands::ai::normalize_from_paste,
            commands::ai::normalize_from_query,
            commands::ai::explain_formula,
            commands::ai::refine_formula,
            commands::ai::resolve_images,
            commands::ai::ai_cancel,
            // ---- 组 14：本地备份 ----
            commands::backup::backup_local_export,
            commands::backup::backup_inspect,
            commands::backup::backup_local_import,
            // ---- 组 14：WebDAV 云端备份 ----
            commands::webdav::webdav_config_get,
            commands::webdav::webdav_config_save,
            commands::webdav::webdav_has_password,
            commands::webdav::webdav_clear_password,
            commands::webdav::webdav_test_connection,
            commands::webdav::webdav_list,
            commands::webdav::webdav_upload,
            commands::webdav::webdav_inspect,
            commands::webdav::webdav_download_import,
            commands::webdav::webdav_delete,
            commands::webdav::webdav_cancel,
            // ---- 组 14：图片缓存 ----
            commands::image::image_list,
            commands::image::image_total_size,
            commands::image::image_delete,
            commands::image::image_load_data_url,
            commands::image::image_save_from_path,
            commands::image::image_save_from_bytes,
            // ---- 组 14：重置软件 ----
            commands::reset::reset_counts,
            commands::reset::reset_confirm_lines,
            commands::reset::reset_execute,
            // ---- 组 14：更新检查 ----
            commands::update::update_check,
            commands::update::update_open_download,
            // ---- 组 10：模型测试 ----
            commands::model_test::model_test_send,
            commands::model_test::model_test_conversation,
            commands::model_test::model_test_clear,
            commands::model_test::model_test_set_system_prompt,
            commands::model_test::model_test_compress,
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 应用失败");
}
