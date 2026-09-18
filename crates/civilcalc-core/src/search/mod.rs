//! 本地公式检索（替代源项目 `core/search/`）。
//!
//! 源：`LocalFormulaIndex.kt`（54 行）+ `PinyinIndex.kt`（43 行）
//!
//! | 文件 | 源文件 | 任务 | 状态 |
//! |---|---|---|---|
//! | `index.rs` | `LocalFormulaIndex.kt` | P2-4 | ✅ 已移植（评分逐条复刻 + 预计算语料） |
//! | `pinyin.rs` | `PinyinIndex.kt` | P2-4 | ✅ 已移植（**修正源项目首字母检索失效的 bug**） |
//!
//! ## 拼音依赖
//!
//! 源项目用 `pinyin4j`；PC 端用 `pinyin` crate（ADR-008）。
//!
//! ## 两处刻意的行为差异（都已文档化）
//!
//! 1. **首字母检索**：源项目 `getFirstLetters` 返回大写而 `match` 用小写比对，
//!    导致 `"lzjm"` 搜不到「梁正截面」（功能失效）。PC 端返回小写，**修正**。
//! 2. **预计算语料**：源项目每次搜索都现场算拼音；PC 端构建索引时算一次并缓存。
//!    **评分规则不变**。
//!
//! 详见各自模块的文档。

pub mod index;
pub mod pinyin;

pub use index::{build_search_text, SearchIndex, SearchSuggestion};
pub use pinyin::{all_variants, first_letters, full_pinyin, matches, pinyin_array};
