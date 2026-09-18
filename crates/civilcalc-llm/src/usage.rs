//! LLM 用量（token 统计）与**跨协议归一化**。
//!
//! 源：`civilcalc-android-v2/core/llm/LlmClient.kt`（`Usage` / `TokenDetails`
//! 及文件末尾的扩展函数，约 100 行）
//!
//! ## 🔴 为什么必须归一化
//!
//! 两套协议的字段命名**完全不同**（这是跨协议差异，不是旧版本兼容）：
//!
//! | 协议 | 输入 | 输出 | 总计 | 缓存命中 |
//! |---|---|---|---|---|
//! | `/chat/completions` | `prompt_tokens` | `completion_tokens` | `total_tokens` | 顶层 `prompt_cache_hit_tokens`（DeepSeek）或 `prompt_tokens_details.cached_tokens`（GLM/MiMo） |
//! | `/responses` | `input_tokens` | `output_tokens` | `total_tokens` | `input_tokens_details.cached_tokens` |
//!
//! ⚠️ **`/responses` 实测不返回 `prompt_tokens` / `completion_tokens`**。
//! 若直接入库，会出现「输入 0、输出 0、总计 1169」这种自相矛盾的记录。
//! 所以读取方一律先调 [`Usage::normalized`]，不必关心协议。
//!
//! ## 缓存命中：三种命名 + miss 反推
//!
//! 命中率必须**唯一口径**，否则同一份数据在不同厂商下算出不同值。
//! 优先级见 [`Usage::cached_tokens`]。

use serde::{Deserialize, Serialize};

/// Token 明细（缓存 / 思考）。
///
/// 四种前缀（`prompt_tokens_details` / `completion_tokens_details` /
/// `input_tokens_details` / `output_tokens_details`）共用同一形状。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenDetails {
    #[serde(default)]
    pub cached_tokens: Option<i64>,
    #[serde(default)]
    pub reasoning_tokens: Option<i64>,
}

/// LLM 用量。字段名**保持 API 原样**（snake_case），不做 rename ——
/// 这里直接反序列化厂商响应，改名会与 JSON 对不上。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Usage {
    // ---- /chat/completions ----
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,

    // ---- 顶层直给的缓存字段（DeepSeek 官方 schema / GLM 实测）----
    #[serde(default)]
    pub prompt_cache_hit_tokens: Option<i64>,
    #[serde(default)]
    pub prompt_cache_miss_tokens: Option<i64>,
    /// 兼容其他命名
    #[serde(default)]
    pub cache_read_input_tokens: Option<i64>,

    // ---- /responses ----
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: Option<i64>,

    // ---- 明细 ----
    #[serde(default)]
    pub prompt_tokens_details: Option<TokenDetails>,
    #[serde(default)]
    pub completion_tokens_details: Option<TokenDetails>,
    #[serde(default)]
    pub input_tokens_details: Option<TokenDetails>,
    #[serde(default)]
    pub output_tokens_details: Option<TokenDetails>,
}

impl Usage {
    /// 归一化到 `prompt/completion/total`（**缺失的才回退，不覆盖已有值**）。
    ///
    /// - `prompt`：`prompt_tokens` > 0 时用它，否则用 `input_tokens`（缺则 0）
    /// - `completion`：`completion_tokens` > 0 时用它，否则用 `output_tokens`（缺则 0）
    /// - `total`：`total_tokens` > 0 时用它，否则 `prompt + completion`
    ///
    /// ⚠️ 「不覆盖已有值」很重要：某些厂商同时返回两套字段且**数值不同**
    /// （如 `prompt_tokens` 已含缓存），用 `input_tokens` 覆盖会改小输入量。
    #[must_use]
    pub fn normalized(&self) -> Usage {
        let prompt = if self.prompt_tokens > 0 {
            self.prompt_tokens
        } else {
            self.input_tokens.unwrap_or(0)
        };
        let completion = if self.completion_tokens > 0 {
            self.completion_tokens
        } else {
            self.output_tokens.unwrap_or(0)
        };
        let total = if self.total_tokens > 0 {
            self.total_tokens
        } else {
            prompt + completion
        };

        if prompt == self.prompt_tokens
            && completion == self.completion_tokens
            && total == self.total_tokens
        {
            self.clone()
        } else {
            Usage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: total,
                ..self.clone()
            }
        }
    }

    /// 缓存命中 token —— 三种命名 + miss 反推，**命中率的唯一口径**。
    ///
    /// 优先级（逐个回退）：
    /// 1. 顶层 `prompt_cache_hit_tokens`（DeepSeek）
    /// 2. `prompt_tokens_details.cached_tokens`（GLM / MiMo）
    /// 3. `input_tokens_details.cached_tokens`（/responses）
    /// 4. `cache_read_input_tokens`（其他命名）
    /// 5. 由 `prompt_cache_miss_tokens` 反推：`prompt_tokens - miss`（下限 0）
    #[must_use]
    pub fn cached_tokens(&self) -> i64 {
        if let Some(v) = self.prompt_cache_hit_tokens {
            return v;
        }
        if let Some(v) = self.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens) {
            return v;
        }
        if let Some(v) = self.input_tokens_details.as_ref().and_then(|d| d.cached_tokens) {
            return v;
        }
        if let Some(v) = self.cache_read_input_tokens {
            return v;
        }
        if let Some(miss) = self.prompt_cache_miss_tokens {
            return (self.prompt_tokens - miss).max(0);
        }
        0
    }

    /// 未命中（按输入计费）的 token。
    #[must_use]
    pub fn uncached_tokens(&self) -> i64 {
        if let Some(v) = self.prompt_cache_miss_tokens {
            return v;
        }
        (self.prompt_tokens - self.cached_tokens()).max(0)
    }

    /// 思考 token（`completion_tokens_details` 优先，回退 `output_tokens_details`）。
    #[must_use]
    pub fn reasoning_tokens(&self) -> i64 {
        self.completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens)
            .or_else(|| {
                self.output_tokens_details
                    .as_ref()
                    .and_then(|d| d.reasoning_tokens)
            })
            .unwrap_or(0)
    }

    /// 缓存命中率（0.0~1.0）；总输入为 0 时返回 `None`（不是 0）。
    ///
    /// 分母用 `cached + uncached`（而非 `prompt_tokens`）—— 某些厂商的
    /// `prompt_tokens` 不含缓存部分，用它当分母会算出 >100% 的命中率。
    #[must_use]
    pub fn hit_rate(&self) -> Option<f32> {
        let total = self.cached_tokens() + self.uncached_tokens();
        if total > 0 {
            Some(self.cached_tokens() as f32 / total as f32)
        } else {
            None
        }
    }
}

/// 合并两次调用的用量（主调用 + JSON 自纠重试）。
///
/// 任一为 `None` 则取另一个；`Option` 字段逐项相加（都 `None` 则仍 `None`，
/// **不把 `None` 当成 0** —— 「厂商没返回」与「确实是 0」是两回事）。
#[must_use]
pub fn merge_usage(a: Option<&Usage>, b: Option<&Usage>) -> Option<Usage> {
    let (a, b) = match (a, b) {
        (None, None) => return None,
        (Some(a), None) => return Some(a.clone()),
        (None, Some(b)) => return Some(b.clone()),
        (Some(a), Some(b)) => (a, b),
    };

    fn sum(x: Option<i64>, y: Option<i64>) -> Option<i64> {
        match (x, y) {
            (None, None) => None,
            (x, y) => Some(x.unwrap_or(0) + y.unwrap_or(0)),
        }
    }
    fn sum_td(x: Option<&TokenDetails>, y: Option<&TokenDetails>) -> Option<TokenDetails> {
        match (x, y) {
            (None, None) => None,
            (x, y) => Some(TokenDetails {
                cached_tokens: sum(
                    x.and_then(|d| d.cached_tokens),
                    y.and_then(|d| d.cached_tokens),
                ),
                reasoning_tokens: sum(
                    x.and_then(|d| d.reasoning_tokens),
                    y.and_then(|d| d.reasoning_tokens),
                ),
            }),
        }
    }

    Some(Usage {
        prompt_tokens: a.prompt_tokens + b.prompt_tokens,
        completion_tokens: a.completion_tokens + b.completion_tokens,
        total_tokens: a.total_tokens + b.total_tokens,
        prompt_cache_hit_tokens: sum(a.prompt_cache_hit_tokens, b.prompt_cache_hit_tokens),
        prompt_cache_miss_tokens: sum(a.prompt_cache_miss_tokens, b.prompt_cache_miss_tokens),
        cache_read_input_tokens: sum(a.cache_read_input_tokens, b.cache_read_input_tokens),
        input_tokens: sum(a.input_tokens, b.input_tokens),
        output_tokens: sum(a.output_tokens, b.output_tokens),
        prompt_tokens_details: sum_td(
            a.prompt_tokens_details.as_ref(),
            b.prompt_tokens_details.as_ref(),
        ),
        completion_tokens_details: sum_td(
            a.completion_tokens_details.as_ref(),
            b.completion_tokens_details.as_ref(),
        ),
        input_tokens_details: sum_td(
            a.input_tokens_details.as_ref(),
            b.input_tokens_details.as_ref(),
        ),
        output_tokens_details: sum_td(
            a.output_tokens_details.as_ref(),
            b.output_tokens_details.as_ref(),
        ),
    })
}

/// 面向 UI / 写库的归一化用量摘要（camelCase，与 `types/system.ts` 的汇总字段同口径）。
///
/// 与 [`Usage`] 的区别：`Usage` 是厂商原始返回（字段名随厂商），
/// 这里是**统一后的整数口径**，供 UI 展示与入库行构造共用。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
}

impl From<&Usage> for NormalizeUsage {
    fn from(u: &Usage) -> Self {
        Self {
            prompt_tokens: u.normalized().prompt_tokens,
            completion_tokens: u.normalized().completion_tokens,
            total_tokens: u.normalized().total_tokens,
            cached_tokens: u.cached_tokens(),
            reasoning_tokens: u.reasoning_tokens(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> Usage {
        serde_json::from_str(s).expect("Usage 反序列化失败")
    }

    // ---------------------------------------------------------------------
    // normalized：跨协议归一化
    // ---------------------------------------------------------------------

    /// `/chat/completions` 原生字段 → 原样保留
    #[test]
    fn normalized_keeps_chat_completions_native() {
        let u = json(r#"{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}"#);
        let n = u.normalized();
        assert_eq!(n.prompt_tokens, 100);
        assert_eq!(n.completion_tokens, 50);
        assert_eq!(n.total_tokens, 150);
    }

    /// `/responses` 的 input/output → 映射到 prompt/completion
    #[test]
    fn normalized_maps_responses_input_output() {
        let u = json(r#"{"input_tokens":1200,"output_tokens":300,"total_tokens":1500}"#);
        let n = u.normalized();
        assert_eq!(n.prompt_tokens, 1200, "input_tokens → prompt_tokens");
        assert_eq!(n.completion_tokens, 300, "output_tokens → completion_tokens");
        assert_eq!(n.total_tokens, 1500);
    }

    /// 🔴 `/responses` **只给 total** 时，输入/输出仍是 0 —— 这是已知缺口，不臆造
    ///
    /// 源注释点名过这个陷阱：直接入库会出现「输入 0、输出 0、总计 1169」。
    /// 本测试把它**钉住**：确实如此，且 `total` 不被改写。
    #[test]
    fn normalized_keeps_zero_io_when_only_total_given() {
        let u = json(r#"{"total_tokens":1169}"#);
        let n = u.normalized();
        assert_eq!(n.prompt_tokens, 0);
        assert_eq!(n.completion_tokens, 0);
        assert_eq!(n.total_tokens, 1169, "total 是厂商给的，不得改写");
    }

    /// `total` 缺失时由 prompt+completion 推导
    #[test]
    fn normalized_derives_total_when_missing() {
        let u = json(r#"{"prompt_tokens":10,"completion_tokens":5}"#);
        assert_eq!(u.normalized().total_tokens, 15);
    }

    /// 🔴 已有 `prompt_tokens` 时**不得**被 `input_tokens` 覆盖
    #[test]
    fn normalized_never_overrides_existing_prompt() {
        let u = json(r#"{"prompt_tokens":100,"input_tokens":7,"completion_tokens":50,"output_tokens":9,"total_tokens":150}"#);
        let n = u.normalized();
        assert_eq!(n.prompt_tokens, 100, "prompt_tokens 优先，不能被 input_tokens 改小");
        assert_eq!(n.completion_tokens, 50, "completion_tokens 优先");
    }

    /// 空对象 → 全 0（防御：厂商返回 `{}` 时不该 panic）
    #[test]
    fn normalized_handles_empty_object() {
        let n = json("{}").normalized();
        assert_eq!(n.prompt_tokens, 0);
        assert_eq!(n.completion_tokens, 0);
        assert_eq!(n.total_tokens, 0);
    }

    /// 未知字段被忽略（厂商加字段不该让解析失败）
    #[test]
    fn unknown_fields_ignored() {
        let u = json(r#"{"prompt_tokens":1,"brand_new_field":{"x":1}}"#);
        assert_eq!(u.normalized().prompt_tokens, 1);
    }

    // ---------------------------------------------------------------------
    // cached_tokens：三种命名 + miss 反推
    // ---------------------------------------------------------------------

    #[test]
    fn cached_prefers_top_level_hit() {
        let u = json(r#"{"prompt_tokens":100,"prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20}"#);
        assert_eq!(u.cached_tokens(), 80);
        assert_eq!(u.uncached_tokens(), 20);
    }

    #[test]
    fn cached_falls_back_to_prompt_details() {
        let u = json(r#"{"prompt_tokens":100,"prompt_tokens_details":{"cached_tokens":64}}"#);
        assert_eq!(u.cached_tokens(), 64, "GLM/MiMo 的 details 命名");
        assert_eq!(u.uncached_tokens(), 36, "无 miss 字段时按 prompt - cached 反推");
    }

    #[test]
    fn cached_falls_back_to_input_details() {
        let u = json(r#"{"prompt_tokens":100,"input_tokens_details":{"cached_tokens":30}}"#);
        assert_eq!(u.cached_tokens(), 30, "/responses 的命名");
    }

    #[test]
    fn cached_falls_back_to_cache_read() {
        let u = json(r#"{"prompt_tokens":100,"cache_read_input_tokens":12}"#);
        assert_eq!(u.cached_tokens(), 12);
    }

    #[test]
    fn cached_derived_from_miss() {
        let u = json(r#"{"prompt_tokens":100,"prompt_cache_miss_tokens":25}"#);
        assert_eq!(u.cached_tokens(), 75, "100 - 25");
        assert_eq!(u.uncached_tokens(), 25);
    }

    /// miss 大于 prompt 时下限 0（不出现负数命中）
    #[test]
    fn cached_derived_never_negative() {
        let u = json(r#"{"prompt_tokens":10,"prompt_cache_miss_tokens":99}"#);
        assert_eq!(u.cached_tokens(), 0);
        assert_eq!(u.uncached_tokens(), 99);
    }

    #[test]
    fn cached_zero_when_nothing_given() {
        let u = json(r#"{"prompt_tokens":100}"#);
        assert_eq!(u.cached_tokens(), 0);
        assert_eq!(u.uncached_tokens(), 100);
    }

    /// 优先级：顶层 hit 胜过 details
    #[test]
    fn cached_priority_top_level_wins() {
        let u = json(r#"{"prompt_tokens":100,"prompt_cache_hit_tokens":50,"prompt_tokens_details":{"cached_tokens":70}}"#);
        assert_eq!(u.cached_tokens(), 50, "顶层字段优先于 details");
    }

    // ---------------------------------------------------------------------
    // reasoning tokens
    // ---------------------------------------------------------------------

    #[test]
    fn reasoning_from_completion_details() {
        let u = json(r#"{"completion_tokens":200,"completion_tokens_details":{"reasoning_tokens":150}}"#);
        assert_eq!(u.reasoning_tokens(), 150);
    }

    #[test]
    fn reasoning_from_output_details() {
        let u = json(r#"{"output_tokens_details":{"reasoning_tokens":77}}"#);
        assert_eq!(u.reasoning_tokens(), 77, "回退 /responses 的命名");
    }

    #[test]
    fn reasoning_prefers_completion_details() {
        let u = json(r#"{"completion_tokens_details":{"reasoning_tokens":10},"output_tokens_details":{"reasoning_tokens":20}}"#);
        assert_eq!(u.reasoning_tokens(), 10);
    }

    #[test]
    fn reasoning_zero_when_absent() {
        assert_eq!(json(r#"{"completion_tokens":5}"#).reasoning_tokens(), 0);
    }

    // ---------------------------------------------------------------------
    // hit_rate
    // ---------------------------------------------------------------------

    #[test]
    fn hit_rate_half() {
        let u = json(r#"{"prompt_tokens":100,"prompt_cache_hit_tokens":50,"prompt_cache_miss_tokens":50}"#);
        assert_eq!(u.hit_rate(), Some(0.5));
    }

    #[test]
    fn hit_rate_full_and_zero() {
        let full = json(r#"{"prompt_tokens":100,"prompt_cache_hit_tokens":100,"prompt_cache_miss_tokens":0}"#);
        assert_eq!(full.hit_rate(), Some(1.0));
        let zero = json(r#"{"prompt_tokens":100,"prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":100}"#);
        assert_eq!(zero.hit_rate(), Some(0.0));
    }

    /// 没有任何输入 → `None`（不是 0，避免展示成「命中率 0%」误导）
    #[test]
    fn hit_rate_none_when_no_input() {
        assert_eq!(json("{}").hit_rate(), None);
        assert_eq!(json(r#"{"total_tokens":9}"#).hit_rate(), None);
    }

    /// 分母用 cached+uncached，不是 prompt_tokens（防 >100%）
    #[test]
    fn hit_rate_denominator_is_cached_plus_uncached() {
        // 某厂商 prompt_tokens 不含缓存部分：命中 90、未命中 10
        let u = json(r#"{"prompt_tokens":10,"prompt_cache_hit_tokens":90,"prompt_cache_miss_tokens":10}"#);
        assert_eq!(u.hit_rate(), Some(0.9), "若用 prompt_tokens 作分母会算出 9.0");
    }

    // ---------------------------------------------------------------------
    // merge_usage
    // ---------------------------------------------------------------------

    #[test]
    fn merge_none_both() {
        assert_eq!(merge_usage(None, None), None);
    }

    #[test]
    fn merge_one_side() {
        let a = json(r#"{"prompt_tokens":1}"#);
        assert_eq!(merge_usage(Some(&a), None), Some(a.clone()));
        assert_eq!(merge_usage(None, Some(&a)), Some(a));
    }

    #[test]
    fn merge_sums_all_scalar_fields() {
        let a = json(r#"{"prompt_tokens":100,"completion_tokens":10,"total_tokens":110,"prompt_cache_hit_tokens":60,"prompt_cache_miss_tokens":40,"input_tokens":100,"output_tokens":10}"#);
        let b = json(r#"{"prompt_tokens":50,"completion_tokens":5,"total_tokens":55,"prompt_cache_hit_tokens":20,"prompt_cache_miss_tokens":30,"input_tokens":50,"output_tokens":5}"#);
        let m = merge_usage(Some(&a), Some(&b)).unwrap();
        assert_eq!(m.prompt_tokens, 150);
        assert_eq!(m.completion_tokens, 15);
        assert_eq!(m.total_tokens, 165);
        assert_eq!(m.prompt_cache_hit_tokens, Some(80));
        assert_eq!(m.prompt_cache_miss_tokens, Some(70));
        assert_eq!(m.input_tokens, Some(150));
        assert_eq!(m.output_tokens, Some(15));
    }

    /// `None` 与 `Some` 相加：`None` 当 0，但**两边都 None 时结果仍是 None**
    #[test]
    fn merge_details_none_semantics() {
        let a = json(r#"{"prompt_tokens":1,"prompt_tokens_details":{"cached_tokens":5}}"#);
        let b = json(r#"{"prompt_tokens":2}"#);
        let m = merge_usage(Some(&a), Some(&b)).unwrap();
        let d = m.prompt_tokens_details.expect("一边有就应保留");
        assert_eq!(d.cached_tokens, Some(5), "None 当 0");
        assert_eq!(d.reasoning_tokens, None, "两边都 None 则仍 None");
    }

    /// 两边都无 details → 结果无 details（不凭空造 `TokenDetails::default()`）
    #[test]
    fn merge_no_details_stays_none() {
        let a = json(r#"{"prompt_tokens":1}"#);
        let b = json(r#"{"prompt_tokens":2}"#);
        let m = merge_usage(Some(&a), Some(&b)).unwrap();
        assert!(m.prompt_tokens_details.is_none());
        assert!(m.completion_tokens_details.is_none());
    }

    /// 合并后可直接归一化（自纠重试场景：主调用 + 重试一起入库）
    #[test]
    fn merged_usage_normalizes() {
        let a = json(r#"{"input_tokens":100,"output_tokens":20}"#);
        let b = json(r#"{"input_tokens":30,"output_tokens":5}"#);
        let m = merge_usage(Some(&a), Some(&b)).unwrap().normalized();
        assert_eq!(m.prompt_tokens, 130);
        assert_eq!(m.completion_tokens, 25);
        assert_eq!(m.total_tokens, 155);
    }

    // ---------------------------------------------------------------------
    // 序列化（用量要写库，键名必须是 snake_case 原样）
    // ---------------------------------------------------------------------

    #[test]
    fn serializes_with_api_field_names() {
        let u = json(r#"{"prompt_tokens":1,"prompt_cache_hit_tokens":2}"#);
        let v = serde_json::to_value(&u).unwrap();
        assert!(v.get("prompt_tokens").is_some(), "不得改成 camelCase");
        assert!(v.get("promptCacheHitTokens").is_none());
    }

    /// 往返一致
    #[test]
    fn roundtrip_stable() {
        let u = json(r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17,"completion_tokens_details":{"reasoning_tokens":6}}"#);
        let back: Usage = serde_json::from_str(&serde_json::to_string(&u).unwrap()).unwrap();
        assert_eq!(back, u);
    }
}
