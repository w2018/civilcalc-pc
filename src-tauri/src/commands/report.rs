//! 组 13：报告与导出（5 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `export_options_get` | — | `ExportOptions` |
//! | `export_options_save` | `options` | `void` |
//! | `report_preview` | `formulaId`, `inputs`, `options?` | `string`（HTML） |
//! | `report_export_docx` | `formulaId`, `inputs`, `options?`, `target` | `DocumentExportResult` |
//! | `report_reveal` | `path` | `void` |
//!
//! **没有 `template_*` 命令** —— 模板 CRUD 已被「导出选项」取代（ADR-023）。
//!
//! ## 三个命令共用的准备步骤
//!
//! [`prepare`]：取公式 → 求值 → 分步求值。
//!
//! ⚠️ **不写计算历史** —— 预览与导出都不是「确认计算」。
//! 用户点 5 次预览就灌 5 条历史，那是 bug 不是功能。
//!
//! ## 三档落盘（ADR-018 / `docs/04` §5.3）
//!
//! | 目标 | 位置 | 是否自动加序号 |
//! |---|---|---|
//! | `privateCache` | `app_cache/reports`（可清理） | ✅ |
//! | `scopedDownloads` | 系统「下载」 | ✅ |
//! | `chosenDirectory` | 原生目录对话框选的目录 | ✅ |
//! | `chosenFile` | 原生保存对话框选的**完整路径** | ❌ 见下 |
//!
//! ⚠️ `chosenFile` **不自动加序号**：用户已经在保存对话框里确认过覆盖，
//! 再悄悄改成别的名字会让他找不到刚存的文件。
//! 另三档是应用自己起名，才需要防覆盖。
//!
//! ⚠️ 序号**从 `(2)` 起**（`报告(2).docx`），不是 `(1)` ——
//! 这是 `export.rs::unique_file_name_in` 的既有行为（P1-9 已测），
//! 与源项目一致。别以为它写错了。
//!
//! ## 原子写
//!
//! 临时文件写在**目标同目录**（`.name.docx.tmp-<uuid>`）再 `rename` ——
//! 这样保证同卷，`rename` 才是原子的。
//! 若写在 `app_cache/tmp` 再 rename，跨盘时 Windows 会返回
//! `ERROR_NOT_SAME_DEVICE`，退化成「复制 + 删」，就不再原子了。
//! 失败一律**删掉临时文件**。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use civilcalc_core::engine::{build_eval_context, eval_multi};
use civilcalc_core::schema::{EvalResult, FormulaSchema, StepResult};
use civilcalc_report::{
    render_html, DocxGenerator, DocumentExportResult, DocumentOutputTarget, ExportOptions,
    ReportError,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::error::{CmdResult, CommandError};
use crate::export::{ensure_within, safe_output_path, sanitize_file_name};
use crate::paths::DesktopPaths;
use crate::state::AppState;

/// 导出进度事件（契约：`{ stage, current, total }`）
pub const EVENT_EXPORT_PROGRESS: &str = "export://progress";

/// 计算书默认文件名（公式名为空时用）
const DEFAULT_REPORT_NAME: &str = "计算书.docx";

// =============================================================================
// 进度事件
// =============================================================================

/// `export://progress` 的载荷。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    /// 阶段名（`"prepare"` / `"render"` / `"write"` / `"done"`）
    pub stage: String,
    pub current: u32,
    pub total: u32,
}

/// 发进度事件。**失败只记日志** —— 进度丢了不该让导出失败。
fn emit_progress(app: &AppHandle, stage: &str, current: u32, total: u32) {
    let payload = ExportProgress {
        stage: stage.to_string(),
        current,
        total,
    };
    if let Err(e) = app.emit(EVENT_EXPORT_PROGRESS, payload) {
        civilcalc_core::log::w("Commands", &format!("发送导出进度失败: {e}"), None);
    }
}

// =============================================================================
// 导出选项
// =============================================================================

/// 读持久化的导出选项（落 `AppConfig`，不是独立表）。
///
/// 解析失败回落默认值（**不报错**）—— 选项丢了不该让用户导不出文件。
fn read_export_options(state: &AppState) -> ExportOptions {
    state
        .with_config(|c| {
            c.get_str_non_empty(crate::config::KEY_EXPORT_OPTIONS)
                .and_then(|json| serde_json::from_str::<ExportOptions>(json).ok())
        })
        .unwrap_or_default()
        .unwrap_or_default()
}

/// 取持久化的导出选项
#[tauri::command]
pub fn export_options_get(state: State<'_, AppState>) -> CmdResult<ExportOptions> {
    Ok(read_export_options(&state))
}

/// 保存导出选项（**整体覆盖**）
#[tauri::command]
pub fn export_options_save(state: State<'_, AppState>, options: ExportOptions) -> CmdResult<()> {
    let json = serde_json::to_string(&options).map_err(|e| CommandError::Parse {
        message: format!("导出选项序列化失败: {e}"),
    })?;
    state.update_config(move |c| c.set_str(crate::config::KEY_EXPORT_OPTIONS, json))
}

// =============================================================================
// 预览
// =============================================================================

/// 渲染 HTML 预览（ADR-018）。
///
/// 返回**完整 HTML 文档**，前端塞进 `iframe.srcdoc` 即可。
///
/// ⚠️ 预览与导出走**两套渲染代码**，存在视觉不一致风险。
/// 预览页已内建标注「预览仅供参考，实际以导出的 Word 文件为准」。
#[tauri::command]
pub fn report_preview(
    state: State<'_, AppState>,
    formula_id: String,
    inputs: HashMap<String, f64>,
    options: Option<ExportOptions>,
) -> CmdResult<String> {
    let options = options.unwrap_or_else(|| read_export_options(&state));
    let (schema, result, steps) = prepare(&state, &formula_id, &inputs)?;

    Ok(render_html(
        &schema,
        &inputs,
        &result,
        &options.to_template(),
        &steps,
    ))
}

// =============================================================================
// 导出
// =============================================================================

/// 导出 `.docx` 计算书。
///
/// 长任务，过程中发 [`EVENT_EXPORT_PROGRESS`]。
///
/// 返回的 `path` 是**最终实际写入的路径** —— 发生同名冲突时会带序号，
/// 前端「打开文件」必须用它。
#[tauri::command]
pub fn report_export_docx(
    app: AppHandle,
    state: State<'_, AppState>,
    formula_id: String,
    inputs: HashMap<String, f64>,
    options: Option<ExportOptions>,
    target: DocumentOutputTarget,
) -> CmdResult<DocumentExportResult> {
    let options = options.unwrap_or_else(|| read_export_options(&state));

    emit_progress(&app, "prepare", 1, 4);
    let (schema, result, steps) = prepare(&state, &formula_id, &inputs)?;

    emit_progress(&app, "render", 2, 4);
    let bytes = DocxGenerator::new()
        .generate(
            &schema,
            &inputs,
            &result,
            &options.to_template(),
            &steps,
        )
        .map_err(map_report_error)?;

    emit_progress(&app, "write", 3, 4);
    let base_name = report_file_name(&schema);
    let path = resolve_output_path(&target, &state.paths, &base_name).map_err(map_report_error)?;
    write_atomic(&path, &bytes).map_err(map_report_error)?;

    let out = DocumentExportResult::new(path.to_string_lossy().to_string(), bytes.len() as i64);
    civilcalc_core::log::i(
        "Commands",
        &format!(
            "导出计算书成功：{}（{} 字节）",
            out.path,
            out.size_bytes
        ),
    );

    emit_progress(&app, "done", 4, 4);
    Ok(out)
}

/// 在系统文件管理器里定位文件。
///
/// 前端「打开所在文件夹」用它。**文件不存在时返回 `notFound`** ——
/// 用户可能刚把它删了，静默什么都不做会让人以为按钮坏了。
#[tauri::command]
pub fn report_reveal(path: String) -> CmdResult<()> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(CommandError::NotFound {
            message: format!("文件不存在：{path}"),
        });
    }
    tauri_plugin_opener::reveal_item_in_dir(&p).map_err(|e| CommandError::Storage {
        message: format!("打开文件所在位置失败：{e}"),
    })
}

// =============================================================================
// 核心逻辑（不依赖 Tauri，便于单测）
// =============================================================================

/// 取公式 → 求值 → 分步求值。
///
/// ⚠️ **不写计算历史**（见模块文档）。
pub(crate) fn prepare(
    state: &AppState,
    formula_id: &str,
    inputs: &HashMap<String, f64>,
) -> CmdResult<(FormulaSchema, EvalResult, Vec<StepResult>)> {
    let schema = state
        .db
        .get_formula(formula_id)?
        .ok_or_else(|| CommandError::NotFound {
            message: format!("公式不存在: {formula_id}"),
        })?;

    let ctx = build_eval_context(&schema, inputs);
    let result = eval_multi(&schema.expression, &schema.constants, &ctx).map_err(|e| {
        CommandError::Parse {
            message: e.to_string(),
        }
    })?;

    // 分步失败**不阻断导出** —— 计算书少一个章节，好过完全导不出。
    // （与 `eval_steps` 命令的语义不同：那里失败就该失败，因为用户专门要看分步。）
    let steps = match schema.steps_template.as_ref().filter(|s| !s.is_empty()) {
        Some(templates) => {
            let variable_order: Vec<String> =
                schema.variables.iter().map(|v| v.symbol.clone()).collect();
            match civilcalc_core::engine::eval_steps(
                templates,
                inputs,
                &schema.constants,
                schema.excel_steps_template.as_deref(),
                &variable_order,
            ) {
                Ok(s) => s,
                Err(e) => {
                    civilcalc_core::log::w(
                        "Commands",
                        &format!("分步求值失败，计算书将不含分步章节（公式 {formula_id}）: {e}"),
                        None,
                    );
                    Vec::new()
                }
            }
        }
        None => Vec::new(),
    };

    Ok((schema, result, steps))
}

/// 计算书文件名（`{公式名}.docx`，公式名为空时用默认名）
fn report_file_name(schema: &FormulaSchema) -> String {
    let name = schema.result_name.trim();
    if name.is_empty() {
        DEFAULT_REPORT_NAME.to_string()
    } else {
        format!("{name}.docx")
    }
}

/// 目标 → 最终落盘路径（含安全校验与同名加序号）。
///
/// 见模块文档的「三档落盘」表。
fn resolve_output_path(
    target: &DocumentOutputTarget,
    paths: &DesktopPaths,
    base_name: &str,
) -> Result<PathBuf, ReportError> {
    match target {
        DocumentOutputTarget::PrivateCache => {
            let dir = paths.reports_dir();
            std::fs::create_dir_all(&dir)?;
            safe_output_path(&dir, base_name).map_err(to_report_error)
        }

        DocumentOutputTarget::ScopedDownloads => {
            let dir = dirs::download_dir().ok_or_else(|| {
                ReportError::Io("系统「下载」目录不可用，请改用「选择目录」".to_string())
            })?;
            safe_output_path(&dir, base_name).map_err(to_report_error)
        }

        DocumentOutputTarget::ChosenDirectory { dir } => {
            let dir = PathBuf::from(dir);
            if !dir.is_dir() {
                return Err(ReportError::Io(format!(
                    "目标目录不存在或不是目录：{}",
                    dir.display()
                )));
            }
            safe_output_path(&dir, base_name).map_err(to_report_error)
        }

        DocumentOutputTarget::ChosenFile { path } => {
            let p = PathBuf::from(path);
            let parent = p.parent().filter(|d| !d.as_os_str().is_empty()).ok_or_else(|| {
                ReportError::Io(format!("目标路径没有父目录：{}", p.display()))
            })?;
            if !parent.is_dir() {
                return Err(ReportError::Io(format!(
                    "目标目录不存在：{}",
                    parent.display()
                )));
            }
            // 文件名仍要清理（用户可能从别处粘贴了带非法字符的名字）
            let raw = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .ok_or_else(|| ReportError::Io(format!("目标路径没有文件名：{}", p.display())))?;
            let clean = sanitize_file_name(&raw);
            let clean = if clean.is_empty() {
                base_name.to_string()
            } else {
                clean
            };
            // 越界校验：解析符号链接后仍须在所选目录内
            ensure_within(parent, &parent.join(&clean)).map_err(to_report_error)
        }
    }
}

/// 原子写：**目标同目录**写临时文件 → `rename`。
///
/// 见模块文档。失败会删掉临时文件，不留垃圾。
fn write_atomic(final_path: &Path, bytes: &[u8]) -> Result<(), ReportError> {
    let dir = final_path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(|| ReportError::Io("目标路径没有父目录".to_string()))?;
    let file_name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| ReportError::Io("目标路径没有文件名".to_string()))?;

    // 临时文件名以 `.` 开头（隐藏）+ uuid 后缀（并发导出不撞车）
    let tmp = dir.join(format!(".{file_name}.tmp-{}", uuid::Uuid::new_v4()));

    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(ReportError::Io(format!(
            "写入临时文件失败（{}）: {e}",
            tmp.display()
        )));
    }

    if let Err(e) = std::fs::rename(&tmp, final_path) {
        // 失败即删草稿 —— 否则目录里会堆一串 `.报告.docx.tmp-xxx`
        let _ = std::fs::remove_file(&tmp);
        return Err(ReportError::Io(format!(
            "重命名失败（{} → {}）: {e}",
            tmp.display(),
            final_path.display()
        )));
    }

    Ok(())
}

/// `CoreError` → `ReportError`（路径校验失败归到 IO）
fn to_report_error(e: civilcalc_core::CoreError) -> ReportError {
    ReportError::Io(e.to_string())
}

/// `ReportError` → `CommandError`（按错误码分流）
fn map_report_error(e: ReportError) -> CommandError {
    let message = e.user_message();
    match e.code() {
        "STORAGE_ERROR" => CommandError::Storage { message },
        "VALIDATION_ERROR" => CommandError::Validation { message },
        _ => CommandError::Export { message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(json: &str) -> FormulaSchema {
        serde_json::from_str(json).expect("schema 应能解析")
    }

    fn base() -> FormulaSchema {
        schema(
            r#"{
                "id":"usr:1","resultName":"矩形面积","resultSymbol":"A",
                "expression":"b*h","source":{"kind":"CUSTOM"}
            }"#,
        )
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("civilcalc-report-test-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // ------------------------------------------------------------ 文件名

    #[test]
    fn file_name_from_result_name() {
        assert_eq!(report_file_name(&base()), "矩形面积.docx");
    }

    #[test]
    fn blank_result_name_falls_back() {
        let mut s = base();
        s.result_name = "   ".to_string();
        assert_eq!(report_file_name(&s), DEFAULT_REPORT_NAME);
    }

    /// 公式名里的非法字符被清理（不能直接拼路径）
    #[test]
    fn illegal_chars_in_name_are_sanitized() {
        let mut s = base();
        s.result_name = "a/b:c".to_string();
        // `report_file_name` 只负责拼名字，清理发生在 `safe_output_path`
        assert_eq!(report_file_name(&s), "a/b:c.docx");
        assert_eq!(sanitize_file_name("a/b:c.docx"), "a_b_c.docx");
    }

    // ------------------------------------------------------------ 目标解析

    #[test]
    fn private_cache_target_creates_dir_and_unique_name() {
        let dir = tmp_dir("private");
        let paths = DesktopPaths::for_test(dir.clone());
        let p = resolve_output_path(
            &DocumentOutputTarget::PrivateCache,
            &paths,
            "报告.docx",
        )
        .unwrap();
        assert!(p.starts_with(&dir));
        assert_eq!(p.file_name().unwrap(), "报告.docx");
    }

    /// 同名时**加序号**（不覆盖）
    #[test]
    fn existing_file_gets_numbered() {
        let dir = tmp_dir("numbered");
        let paths = DesktopPaths::for_test(dir.clone());
        let reports = paths.reports_dir();
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(reports.join("报告.docx"), b"old").unwrap();

        let p = resolve_output_path(&DocumentOutputTarget::PrivateCache, &paths, "报告.docx").unwrap();
        // ⚠️ 序号从 (2) 起（`unique_file_name_in` 的既有行为）
        assert_eq!(p.file_name().unwrap(), "报告(2).docx");
        // 原文件内容未被改动
        assert_eq!(std::fs::read(reports.join("报告.docx")).unwrap(), b"old");
    }

    #[test]
    fn chosen_directory_must_exist() {
        let e = resolve_output_path(
            &DocumentOutputTarget::ChosenDirectory {
                dir: "Z:/definitely/not/here".to_string(),
            },
            &DesktopPaths::for_test(tmp_dir("chosen-missing")),
            "a.docx",
        )
        .unwrap_err();
        assert!(matches!(e, ReportError::Io(_)));
    }

    #[test]
    fn chosen_directory_writes_inside_it() {
        let dir = tmp_dir("chosen");
        let p = resolve_output_path(
            &DocumentOutputTarget::ChosenDirectory {
                dir: dir.to_string_lossy().to_string(),
            },
            &DesktopPaths::for_test(tmp_dir("chosen-unused")),
            "a.docx",
        )
        .unwrap();
        assert_eq!(p.parent().unwrap(), dir.as_path());
    }

    /// `chosenFile` **不加序号**（用户已确认过覆盖）
    #[test]
    fn chosen_file_keeps_exact_name() {
        let dir = tmp_dir("chosen-file");
        let existing = dir.join("a.docx");
        std::fs::write(&existing, b"old").unwrap();

        let p = resolve_output_path(
            &DocumentOutputTarget::ChosenFile {
                path: existing.to_string_lossy().to_string(),
            },
            &DesktopPaths::for_test(tmp_dir("chosen-file-unused")),
            "ignored.docx",
        )
        .unwrap();
        assert_eq!(p, existing, "不得改成 a(1).docx");
    }

    /// 文件名里的非法字符被清理成下划线
    #[test]
    fn chosen_file_sanitizes_name() {
        let dir = tmp_dir("chosen-file-bad");
        let p = resolve_output_path(
            &DocumentOutputTarget::ChosenFile {
                path: dir.join("a?b*.docx").to_string_lossy().to_string(),
            },
            &DesktopPaths::for_test(tmp_dir("chosen-file-bad-unused")),
            "fallback.docx",
        )
        .unwrap();
        assert_eq!(p.file_name().unwrap(), "a_b_.docx", "非法字符换成下划线");
    }

    /// ⚠️ 带**路径分隔符**的名字会先被 `parent()` 解析掉 ——
    /// `dir/a/b.docx` 的父目录是 `dir/a`（不存在）→ 直接报错。
    /// 这是**有意的防御**：不接受「路径里再套路径」。
    #[test]
    fn chosen_file_with_embedded_separator_fails() {
        let dir = tmp_dir("chosen-file-sep");
        let e = resolve_output_path(
            &DocumentOutputTarget::ChosenFile {
                path: dir.join("a/b.docx").to_string_lossy().to_string(),
            },
            &DesktopPaths::for_test(tmp_dir("chosen-file-sep-unused")),
            "fallback.docx",
        )
        .unwrap_err();
        assert!(matches!(e, ReportError::Io(_)), "实际: {e:?}");
    }

    /// 🔴 **Windows 特有陷阱**：文件名里出现 `:` 会被 `Path` 解析成**盘符**。
    ///
    /// `dir.join("a:b.docx")` 的 `parent()` 不是 `dir`，而是 **`a:`** ——
    /// 于是报「目标目录不存在：a:」。
    ///
    /// 结果是对的（这种路径本来就不该接受），但**报错信息会让人困惑**。
    /// 所以这里把它钉住：将来若有人把 `parent()` 判断挪到清理**之后**，
    /// 行为会变（变成 `dir/a_b.docx`），这条测试会失败提醒他。
    #[test]
    fn chosen_file_with_colon_is_rejected_on_windows() {
        let dir = tmp_dir("chosen-file-colon");
        let e = resolve_output_path(
            &DocumentOutputTarget::ChosenFile {
                path: dir.join("a:b.docx").to_string_lossy().to_string(),
            },
            &DesktopPaths::for_test(tmp_dir("chosen-file-colon-unused")),
            "fallback.docx",
        )
        .unwrap_err();
        assert!(matches!(e, ReportError::Io(_)), "实际: {e:?}");
        assert!(
            e.detail().contains("a:"),
            "报错会指向被解析出的盘符，便于定位: {}",
            e.detail()
        );
    }

    #[test]
    fn chosen_file_missing_parent_fails() {
        let e = resolve_output_path(
            &DocumentOutputTarget::ChosenFile {
                path: "Z:/nope/a.docx".to_string(),
            },
            &DesktopPaths::for_test(tmp_dir("chosen-file-noparent")),
            "a.docx",
        )
        .unwrap_err();
        assert!(matches!(e, ReportError::Io(_)));
    }

    // ------------------------------------------------------------ 原子写

    #[test]
    fn atomic_write_creates_file() {
        let dir = tmp_dir("atomic");
        let f = dir.join("out.docx");
        write_atomic(&f, b"PK\x03\x04hello").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"PK\x03\x04hello");
    }

    /// 写成功后**不留临时文件**
    #[test]
    fn atomic_write_leaves_no_temp() {
        let dir = tmp_dir("atomic-clean");
        write_atomic(&dir.join("out.docx"), b"data").unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "不应残留临时文件: {leftovers:?}");
    }

    /// 覆盖已有文件（`chosenFile` 场景）
    #[test]
    fn atomic_write_overwrites() {
        let dir = tmp_dir("atomic-overwrite");
        let f = dir.join("out.docx");
        std::fs::write(&f, b"old").unwrap();
        write_atomic(&f, b"new").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"new");
    }

    /// 目标目录不存在 → 报错且**不创建**临时文件
    #[test]
    fn atomic_write_to_missing_dir_fails_cleanly() {
        let dir = tmp_dir("atomic-missing");
        let e = write_atomic(&dir.join("nope/out.docx"), b"x").unwrap_err();
        assert!(matches!(e, ReportError::Io(_)));
        assert!(!dir.join("nope").exists(), "不该顺手创建目录");
    }

    // ------------------------------------------------------------ 错误映射

    #[test]
    fn report_error_maps_to_command_error() {
        assert_eq!(
            map_report_error(ReportError::Io("x".into())).kind_str(),
            "storage"
        );
        assert_eq!(
            map_report_error(ReportError::Template("x".into())).kind_str(),
            "validation"
        );
        assert_eq!(
            map_report_error(ReportError::Docx("x".into())).kind_str(),
            "export"
        );
        assert_eq!(
            map_report_error(ReportError::Unsupported("x".into())).kind_str(),
            "export"
        );
    }

    /// 用户文案不含内部细节
    #[test]
    fn command_error_hides_internals() {
        let e = map_report_error(ReportError::Docx("zip panicked at 42".into()));
        assert!(!e.to_string().contains("42"));
    }

    // ------------------------------------------------------------ 进度载荷

    #[test]
    fn progress_payload_shape() {
        let v = serde_json::to_value(ExportProgress {
            stage: "render".to_string(),
            current: 2,
            total: 4,
        })
        .unwrap();
        assert_eq!(v["stage"], serde_json::json!("render"));
        assert_eq!(v["current"], serde_json::json!(2));
        assert_eq!(v["total"], serde_json::json!(4));
    }

    #[test]
    fn event_name_matches_contract() {
        assert_eq!(EVENT_EXPORT_PROGRESS, "export://progress");
    }

    // ------------------------------------------------------------ 导出选项

    #[test]
    fn export_options_roundtrip_json() {
        let o = ExportOptions {
            show_notes: false,
            disclaimer: "仅供内部复核。".to_string(),
            ..ExportOptions::default()
        };
        let json = serde_json::to_string(&o).unwrap();
        let back: ExportOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }

    /// 坏 JSON 应回落默认值（不 panic、不报错）
    #[test]
    fn broken_options_json_falls_back() {
        let parsed: Option<ExportOptions> = serde_json::from_str("{not json").ok();
        assert!(parsed.is_none());
    }

    // ------------------------------------------------------------ 文件名与目标联动

    /// 端到端：从 schema 到最终路径
    #[test]
    fn end_to_end_path_resolution() {
        let dir = tmp_dir("e2e");
        let paths = DesktopPaths::for_test(dir.clone());
        let name = report_file_name(&base());
        let p = resolve_output_path(&DocumentOutputTarget::PrivateCache, &paths, &name).unwrap();
        assert!(p.to_string_lossy().ends_with("矩形面积.docx"));

        write_atomic(&p, b"PK\x03\x04").unwrap();
        assert!(p.exists());

        // 再导一次 → 加序号（从 (2) 起）
        let p2 = resolve_output_path(&DocumentOutputTarget::PrivateCache, &paths, &name).unwrap();
        assert_ne!(p2, p);
        assert!(p2.to_string_lossy().ends_with("矩形面积(2).docx"));
    }
}
