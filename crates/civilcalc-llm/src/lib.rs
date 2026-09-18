//! # civilcalc-llm —— LLM 层
//!
//! 源：`civilcalc-android-v2/core/llm/`（7 文件 1,355 行）
//!
//! ## 硬约束
//!
//! - **不得依赖 `tauri`**（ADR-002）
//!
//! ## 依赖方向（P4-4 修正）
//!
//! `llm → civilcalc-core` 是**允许且必需**的（`normalizer` 要产出 `FormulaSchema`）。
//! 真正的约束是**不反向**：`civilcalc-core` 不依赖 `llm`（core 不含 HTTP）。
//! 本文件原先写「不得依赖 civilcalc-core（避免循环）」——与 `docs/02` 的依赖图矛盾，已修正。
//!
//! ## 模块对应
//!
//! | 本 crate 模块 | 源文件 | 任务 |
//! |---|---|---|
//! | `config.rs` | `LlmConfig.kt` | P4-1 |
//! | `error.rs` | `LlmError.kt` | ✅ 已完成（P1-5 提前：`CommandError` 需要它） |
//! | `prompts.rs` | `LlmPrompts.kt` | P4-1 |
//! | `client.rs` | `LlmClient.kt` | P4-2 |
//! | `protocol.rs` | `LlmClient.kt`（协议部分） | P4-2 |
//! | `stream.rs` | `LlmClient.kt`（流式部分） | P4-2 |
//! | `usage.rs` | `LlmClient.kt`（Usage 部分） | P4-3 |
//! | `normalizer.rs` | `LlmNormalizer.kt` | P4-4 |
//! | `explainer.rs` | `FormulaExplainer.kt` | P4-5 |
//! | `refine.rs` | `FormulaRefine.kt` | P4-5 |
//!
//! > 任务号说明：`error.rs` 原计划在 P4-1，因 **P1-5（`CommandError` 的三个 `From`）**
//! > 需要 `LlmError` 类型而提前实现。其余模块仍按 P4 顺序。
//!
//! ## ⚠️ 必须保留的"实测知识"（重写最易丢失的资产）
//!
//! 1. **GLM glm-5.3 系列强制思考**：发 `thinking.type=disabled` 直接 400（错误码 1210），
//!    只能靠 `reasoning_effort` low/high/max（默认 max）→ **"关闭"须降级为 low**
//! 2. **小米 MiMo 无档位控制**：`thinking.type` 只能开/关
//! 3. **`/responses` 不返回 prompt/completion_tokens**：只有 `total_tokens`，
//!    直接入库会出现「输入 0、输出 0、总计 1169」的自相矛盾记录
//! 4. **缓存字段两套命名**：顶层 `prompt_cache_hit_tokens`（DeepSeek）
//!    vs `prompt_tokens_details.cached_tokens`（GLM/MiMo）
//!
//! 详见 `docs/04-数据契约.md` §6。

pub mod client;
pub mod config;
pub mod error;
pub mod prompts;
pub mod protocol;
pub mod stream;
pub mod usage;

pub mod normalizer;
pub mod explainer;
pub mod refine;

pub use config::{
    default_profiles, effective_thinking_level, forces_thinking, mask_secret,
    supports_reasoning_effort, supports_web_search, ApiProtocol, LlmConfig, LlmProfile,
    ResolvedLlmProfile, ThinkingLevel,
};
pub use client::{thinking_params, use_responses_api, LlmClient, ThinkingParams};
pub use error::{LlmError, LlmErrorCode};
pub use explainer::{build_user_text, LlmExplainer, MAX_EXPLAIN_RETRY};
pub use refine::{build_revise_request, next_version_name};
pub use normalizer::{
    new_id_if_refine, normalize_schema, strip_markdown_fences, LlmNormalizer, NormalizeRequest,
    NormalizeResult, MAX_JSON_RETRY,
};
pub use protocol::{ChatMessage, ChatResult};
pub use stream::Signal;
pub use prompts::{prompt_a, PROMPT_A, PROMPT_A_CORE, PROMPT_A_EXPLAIN, PROMPT_EXPLAIN};
pub use usage::{merge_usage, NormalizeUsage, TokenDetails, Usage};
