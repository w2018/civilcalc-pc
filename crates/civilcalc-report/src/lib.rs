//! # civilcalc-report —— 报告生成
//!
//! 源：`civilcalc-android-v2/core/report/`（5 文件约 600 行）
//!
//! ## 技术替换（ADR-007）
//!
//! **Apache POI 5.3.0（JVM）→ `docx-rs`（Rust）**
//!
//! 源项目 `DocxGenerator` 用 `org.apache.poi.xwpf.usermodel.XWPFDocument`；
//! PC 端用 `docx_rs::Docx`。验收标准为「**内容完整 + 结构正确**」，不追求像素级一致。
//!
//! 顺带收益：源项目因 POI 需额外 ProGuard 保名规则，且 v3.2.3 曾因
//! "release 包缺 POI/log4j 保名规则"导致导出闪退；改用 `docx-rs` 后此问题不存在。
//!
//! ## 模块对应
//!
//! | 本 crate 模块 | 源文件 | 任务 |
//! |---|---|---|
//! | `template_model.rs` | `TemplateModel.kt` | P3-9 |
//! | `export_options.rs` | `ExportOptions.kt` | P3-9 |
//! | `report_text.rs` | `ReportText.kt` | P3-9 |
//! | `export_location.rs` | `ExportLocation.kt` | P3-9 |
//! | `docx.rs` | `DocxGenerator.kt` | P3-10 |
//! | `render.rs` | —（PC 端新增，供 HTML 预览复用，ADR-018） | P3-11 |
//!
//! ## ⚠️ 必须保留的行为
//!
//! 1. 7 个章节的**渲染顺序与标题文案**
//! 2. 默认免责声明**逐字一致**：
//!    `本计算书由AI全能计算器自动生成，结果需经注册工程师复核。`
//! 3. `ReportText` 清洗规则：**去列表前缀必须在拆行之前**
//!    （否则 `- ① …` 会留下孤立的 `-`）
//! 4. 数字格式化对齐源项目 `formatNumber`
//!
//! ## 导出选项取代模板（ADR-023）
//!
//! 源项目已用「导出选项」取代「模板」CRUD（源码注释："用户判定无存在价值"）。
//! PC 端只实现 `ExportOptions`（7 个勾选 + 免责声明），不实现模板管理 UI；
//! 但 `ReportTemplate` / `TemplateSection` 模型保留（`to_template()` 的载体）。

pub mod error;

pub mod docx;
pub mod export_location;
pub mod export_options;
pub mod render;
pub mod report_text;
pub mod template_model;


pub use error::ReportError;
pub use docx::DocxGenerator;
pub use export_location::{human_size, DocumentExportResult, DocumentOutputTarget};
pub use export_options::{has_explanation_content, ExportOptions, TEMPLATE_ID};
pub use civilcalc_core::number_format::format_number;
pub use render::{escape_html, render_html};
pub use report_text::{bold_segments, plain_lines};
pub use civilcalc_core::result_outputs::{
    build_copy_text, build_declared_summary, build_inline_text, is_multi, resolve as resolve_outputs,
    sanitize as sanitize_result_outputs, ResolvedResult,
};
pub use template_model::{
    CoverConfig, CoverField, NotesConfig, ReportTemplate, StepStyle, TemplateSection,
    DEFAULT_DISCLAIMER, DEFAULT_PARAMS_COLUMNS,
};
