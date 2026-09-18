//! 本地公式检索索引（替代源项目 `LocalFormulaIndex.kt`）。
//!
//! ## 与源项目的差异：**预计算语料**
//!
//! 源项目每次 `search()` 都对**全部公式**现场重算检索语料：
//!
//! ```kotlin
//! val all = getAllFormulas()          // 每次都查库
//! val searchText = buildSearchText(formula).lowercase()   // 每次都拼字符串
//! if (pinyinIndex.match(query, formula.resultName)) ...   // 每次都算拼音
//! ```
//!
//! `pinyinIndex.match` 内部对**每个字符**做一次拼音查表。搜索框每敲一个字
//! 就触发一次，几百条公式 × 每条几十字 = 上万次拼音查表 —— 这是不必要的。
//!
//! PC 端把「小写检索文本」与「拼音变体」**在构建索引时算一次**并缓存，
//! 查询时只做 `contains` 比较。这也让 `search_rebuild_index` 命令有了实际语义
//! （源项目那个 lambda 每次现查，没有"索引"可言）。
//!
//! ⚠️ **评分规则逐条复刻**（见 [`SearchIndex::score`]），只改"什么时候算"，
//! 不改"算什么"。
//!
//! ## 评分表（对齐源项目 `calculateScore`）
//!
//! | 条件 | 加分 |
//! |---|---|
//! | 检索文本（名称+领域+标签+变量释义+变量符号+备选标签）含查询 | **100** |
//! | ↳ 且 `resultName` 含查询 | +50 |
//! | ↳ 且 `domain` 含查询 | +30 |
//! | ↳ 且任一 `tag` 含查询 | +20 |
//! | 拼音命中 `resultName` | +40 |
//! | 拼音命中 `domain` | +20 |
//! | 拼音命中每个 `tag` | +15（可累加） |
//!
//! 分数为 0 的条目不返回；结果按分数**降序**，同分保持**原顺序**（稳定排序）。

use crate::schema::{plain_desc, FormulaSchema, SourceKind};
use crate::search::pinyin;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 检索建议（`search_suggest` 的轻量返回，减少 IPC 负载）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSuggestion {
    pub id: String,
    pub result_name: String,
    pub domain: String,
    pub source_kind: SourceKind,
}

/// 一条索引项：公式 + 预计算的检索语料
#[derive(Debug, Clone)]
struct IndexEntry {
    formula: FormulaSchema,

    /// `buildSearchText(formula).to_lowercase()`
    search_text: String,
    /// `formula.result_name.to_lowercase()`
    result_name: String,
    /// `formula.domain.to_lowercase()`
    domain: String,
    /// 各 tag 的小写形式
    tags: Vec<String>,

    /// `pinyin::all_variants(result_name)`
    name_variants: HashSet<String>,
    /// `pinyin::all_variants(domain)`
    domain_variants: HashSet<String>,
    /// 每个 tag 的拼音变体
    tag_variants: Vec<HashSet<String>>,
}

/// 本地公式检索索引。
///
/// 不可变：重建请整体替换（`SearchIndex::build`），避免增量更新引入不一致。
#[derive(Debug, Clone, Default)]
pub struct SearchIndex {
    entries: Vec<IndexEntry>,
}

impl SearchIndex {
    /// 用公式列表构建索引。
    ///
    /// 公式数量是几十到几百量级，全量重建的成本远低于维护增量索引的复杂度。
    pub fn build(formulas: &[FormulaSchema]) -> Self {
        Self {
            entries: formulas.iter().map(IndexEntry::new).collect(),
        }
    }

    /// 索引项数
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 检索，返回按分数降序的公式（分数 0 的不返回）。
    ///
    /// `limit` 为 0 时返回空列表；`query` 为空白时返回空列表
    /// （对齐源项目 `if (q.isBlank()) return emptyList()`）。
    pub fn search(&self, query: &str, limit: usize) -> Vec<FormulaSchema> {
        let q = query.trim().to_lowercase();
        if q.is_empty() || limit == 0 {
            return Vec::new();
        }

        let mut scored: Vec<(usize, i32)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let s = e.score(&q);
                (s > 0).then_some((i, s))
            })
            .collect();

        // 稳定排序：同分保持原顺序（源项目 `sortedByDescending` 也是稳定的）
        scored.sort_by_key(|x| std::cmp::Reverse(x.1));

        scored
            .into_iter()
            .take(limit)
            .map(|(i, _)| self.entries[i].formula.clone())
            .collect()
    }

    /// 轻量建议（只回 id + 名称 + 领域 + 来源），减少 IPC 负载。
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<SearchSuggestion> {
        let q = query.trim().to_lowercase();
        if q.is_empty() || limit == 0 {
            return Vec::new();
        }

        let mut scored: Vec<(usize, i32)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let s = e.score(&q);
                (s > 0).then_some((i, s))
            })
            .collect();
        scored.sort_by_key(|x| std::cmp::Reverse(x.1));

        scored
            .into_iter()
            .take(limit)
            .map(|(i, _)| {
                let f = &self.entries[i].formula;
                SearchSuggestion {
                    id: f.id.clone(),
                    result_name: f.result_name.clone(),
                    domain: f.domain.clone(),
                    source_kind: f.source.kind,
                }
            })
            .collect()
    }

    /// 检索并返回 `(公式, 分数)`，供排障与测试断言评分。
    pub fn search_scored(&self, query: &str, limit: usize) -> Vec<(FormulaSchema, i32)> {
        let q = query.trim().to_lowercase();
        if q.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut scored: Vec<(usize, i32)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let s = e.score(&q);
                (s > 0).then_some((i, s))
            })
            .collect();
        scored.sort_by_key(|x| std::cmp::Reverse(x.1));
        scored
            .into_iter()
            .take(limit)
            .map(|(i, s)| (self.entries[i].formula.clone(), s))
            .collect()
    }
}

impl IndexEntry {
    fn new(formula: &FormulaSchema) -> Self {
        let result_name = formula.result_name.to_lowercase();
        let domain = formula.domain.to_lowercase();
        let tags: Vec<String> = formula.tags.iter().map(|t| t.to_lowercase()).collect();

        let name_variants = pinyin::all_variants(&formula.result_name);
        let domain_variants = pinyin::all_variants(&formula.domain);
        let tag_variants = formula.tags.iter().map(|t| pinyin::all_variants(t)).collect();

        Self {
            search_text: build_search_text(formula).to_lowercase(),
            result_name,
            domain,
            tags,
            name_variants,
            domain_variants,
            tag_variants,
            formula: formula.clone(),
        }
    }

    /// 逐条对齐源项目 `LocalFormulaIndex.calculateScore`。
    ///
    /// `query` **必须已是小写且已 trim**（调用方统一处理）。
    fn score(&self, query: &str) -> i32 {
        let mut score = 0;

        if self.search_text.contains(query) {
            score += 100;
            if self.result_name.contains(query) {
                score += 50;
            }
            if self.domain.contains(query) {
                score += 30;
            }
            if self.tags.iter().any(|t| t.contains(query)) {
                score += 20;
            }
        }

        // 拼音部分：源项目直接调 `pinyinIndex.match`，这里用预计算的变体集合
        if variants_match(&self.name_variants, &self.result_name, query) {
            score += 40;
        }
        if variants_match(&self.domain_variants, &self.domain, query) {
            score += 20;
        }
        for (tv, t) in self.tag_variants.iter().zip(self.tags.iter()) {
            if variants_match(tv, t, query) {
                score += 15;
            }
        }

        score
    }
}

/// 等价于源项目 `PinyinIndex.match(query, target)`，但用预计算的变体集合。
///
/// `target_lower` 是原文小写，`variants` 是 `pinyin::all_variants(原文)`
/// （其中已包含原文小写，但为了与源项目"先查原文再查拼音"的短路顺序一致，
/// 这里显式先查一次）。
fn variants_match(variants: &HashSet<String>, target_lower: &str, query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    target_lower.contains(query) || variants.iter().any(|v| v.contains(query))
}

/// 检索文本（对齐源项目 `buildSearchText`）。
///
/// 顺序：`resultName` → `domain` → 各 `tag` → 各变量的 `plainDesc()` 与 `symbol`
/// → 各备选表达式的 `label`。
///
/// `plainDesc` 会剥掉 `**` / `==` / `` ` `` 这些 Markdown 标记，
/// 否则用户搜"截面"时，描述里的 `**截面**` 会被标记符号隔开而搜不到。
pub fn build_search_text(formula: &FormulaSchema) -> String {
    let mut s = String::with_capacity(128);
    s.push_str(&formula.result_name);
    s.push(' ');
    s.push_str(&formula.domain);
    s.push(' ');
    for t in &formula.tags {
        s.push_str(t);
        s.push(' ');
    }
    for v in &formula.variables {
        s.push_str(&plain_desc(v));
        s.push(' ');
        s.push_str(&v.symbol);
        s.push(' ');
    }
    for a in &formula.alt_expressions {
        s.push_str(&a.label);
        s.push(' ');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FormulaSource, FormulaVar, CURRENT_SCHEMA_VERSION};

    fn var(symbol: &str, desc: &str) -> FormulaVar {
        FormulaVar {
            symbol: symbol.to_string(),
            desc: desc.to_string(),
            unit: Some("mm".to_string()),
            default: None,
            min: None,
            max: None,
            required: true,
        }
    }

    fn formula(id: &str, name: &str, domain: &str, tags: &[&str]) -> FormulaSchema {
        FormulaSchema {
            id: id.to_string(),
            result_name: name.to_string(),
            result_symbol: "y".into(),
            result_unit: None,
            result_outputs: vec![],
            expression: "a+b".into(),
            source_equations: vec![],
            alt_expressions: vec![],
            constants: Default::default(),
            variables: vec![],
            domain: domain.to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: vec![],
            revised_from: None,
            source: FormulaSource::custom(),
            steps_template: None,
            doc_template_id: None,
            excel_expression: None,
            excel_alt_expressions: None,
            excel_steps_template: None,
            excel_function_docs: None,
            schema_version: CURRENT_SCHEMA_VERSION,
            created_at: 1000,
            updated_at: 1000,
        }
    }

    // ---------------- 检索文本 ----------------

    #[test]
    fn search_text_includes_all_sources() {
        let mut f = formula("f", "梁正截面", "结构", &["受弯", "承载力"]);
        f.variables = vec![var("As", "**受拉**钢筋面积")];
        f.alt_expressions = vec![crate::schema::AltExpression {
            label: "简化式".into(),
            expression: "a".into(),
            condition: None,
        }];

        let t = build_search_text(&f);
        assert!(t.contains("梁正截面"));
        assert!(t.contains("结构"));
        assert!(t.contains("受弯"));
        assert!(t.contains("承载力"));
        assert!(t.contains("As"), "变量符号应在");
        assert!(t.contains("受拉钢筋面积"), "变量释义应剥掉 Markdown: {t}");
        assert!(!t.contains("**"), "Markdown 标记应被剥掉");
        assert!(t.contains("简化式"), "备选表达式标签应在");
    }

    // ---------------- 空查询 ----------------

    #[test]
    fn blank_query_returns_empty() {
        let idx = SearchIndex::build(&[formula("f", "梁", "结构", &[])]);
        assert!(idx.search("", 20).is_empty());
        assert!(idx.search("   ", 20).is_empty());
        assert!(idx.suggest("", 20).is_empty());
    }

    #[test]
    fn zero_limit_returns_empty() {
        let idx = SearchIndex::build(&[formula("f", "梁", "结构", &[])]);
        assert!(idx.search("梁", 0).is_empty());
    }

    #[test]
    fn empty_index_returns_empty() {
        let idx = SearchIndex::build(&[]);
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
        assert!(idx.search("梁", 20).is_empty());
    }

    // ---------------- 评分：100 档 ----------------

    /// ⚠️ **中文查询会双重计分**（源项目行为，不是 bug）：
    ///
    /// `pinyinIndex.match` 内部**先查原文小写**再查拼音变体，
    /// 而 `all_variants` 里本来就含原文小写。因此中文查询命中 `resultName` 时：
    ///
    /// - 检索文本含 → 100
    /// - `resultName` 含 → 50
    /// - **拼音匹配（其实是原文比对）也命中** → 40
    ///
    /// = **190**。看着像重复计分，但源项目就是这算法，逐条复刻。
    #[test]
    fn name_hit_gets_190_due_to_double_count() {
        let idx = SearchIndex::build(&[formula("f", "梁正截面", "结构", &[])]);
        let r = idx.search_scored("梁正", 20);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1, 190, "100 + 50 + 40（拼音分支也命中原文）");
    }

    #[test]
    fn domain_hit_gets_150() {
        // searchText 含（100）+ domain 含（30）+ 拼音分支命中 domain 原文（20）
        let idx = SearchIndex::build(&[formula("f", "某某公式", "结构", &[])]);
        let r = idx.search_scored("结构", 20);
        assert_eq!(r[0].1, 150, "100 + 30 + 20");
    }

    #[test]
    fn tag_hit_gets_135() {
        // searchText 含（100）+ tag 含（20）+ 拼音分支命中该 tag 原文（15）
        let idx = SearchIndex::build(&[formula("f", "某某公式", "结构", &["受弯"])]);
        let r = idx.search_scored("受弯", 20);
        assert_eq!(r[0].1, 135, "100 + 20 + 15");
    }

    #[test]
    fn variable_desc_hit_only_gets_100() {
        // 只在变量释义里 → searchText 含（100），但名称/领域/标签都不含
        let mut f = formula("f", "某某公式", "结构", &[]);
        f.variables = vec![var("As", "受拉钢筋面积")];
        let idx = SearchIndex::build(&[f]);
        let r = idx.search_scored("受拉", 20);
        assert_eq!(r[0].1, 100);
    }

    /// tag 的 **+20 只加一次**（源项目用 `any`），
    /// 但**拼音分支的 +15 是 `forEach`，每个命中的 tag 都加**。
    #[test]
    fn multiple_tags_accumulate() {
        let idx = SearchIndex::build(&[formula("f", "某某公式", "结构", &["受弯", "受弯构件"])]);
        let r = idx.search_scored("受弯", 20);
        // 100（检索文本）+ 20（any 一次）+ 15 + 15（两个 tag 各自命中）
        assert_eq!(r[0].1, 150);
    }

    // ---------------- 评分：拼音档 ----------------

    #[test]
    fn pinyin_full_match_on_name_gets_140() {
        // 拼音命中 resultName（40）；searchText 里是中文，不含 "liangzheng"
        let idx = SearchIndex::build(&[formula("f", "梁正截面", "结构", &[])]);
        let r = idx.search_scored("liangzheng", 20);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1, 40, "仅拼音命中名称");
    }

    #[test]
    fn pinyin_first_letters_on_name_gets_40() {
        let idx = SearchIndex::build(&[formula("f", "梁正截面", "结构", &[])]);
        let r = idx.search_scored("lzjm", 20);
        assert_eq!(r[0].1, 40, "首字母缩写应命中（源项目此功能失效）");
    }

    #[test]
    fn pinyin_on_domain_gets_20() {
        let idx = SearchIndex::build(&[formula("f", "某某公式", "结构", &[])]);
        let r = idx.search_scored("jiegou", 20);
        assert_eq!(r[0].1, 20, "仅拼音命中领域");
    }

    #[test]
    fn pinyin_on_tags_accumulates() {
        let idx = SearchIndex::build(&[formula("f", "某某公式", "结构", &["受弯", "承载力"])]);
        let r = idx.search_scored("shouwan", 20);
        assert_eq!(r[0].1, 15, "仅拼音命中一个标签");
    }

    #[test]
    fn pinyin_and_text_scores_add_up() {
        // 名称含"梁"（100+50），拼音也命中（40）
        let idx = SearchIndex::build(&[formula("f", "梁正截面", "结构", &[])]);
        let r = idx.search_scored("liang", 20);
        // searchText 是小写中文，不含 "liang" → 只走拼音
        assert_eq!(r[0].1, 40);
    }

    // ---------------- 排序与截断 ----------------

    #[test]
    fn results_sorted_by_score_desc() {
        let idx = SearchIndex::build(&[
            formula("low", "某某公式", "结构", &[]),      // 领域命中 130
            formula("high", "结构计算", "其他", &[]),     // 名称命中 150
        ]);
        let r = idx.search("结构", 20);
        assert_eq!(r[0].id, "high", "分数高的在前");
        assert_eq!(r[1].id, "low");
    }

    #[test]
    fn zero_score_excluded() {
        let idx = SearchIndex::build(&[formula("f", "梁正截面", "结构", &[])]);
        assert!(idx.search("完全无关的词", 20).is_empty());
    }

    #[test]
    fn limit_truncates() {
        let fs: Vec<FormulaSchema> = (0..10)
            .map(|i| formula(&format!("f{i}"), "梁正截面", "结构", &[]))
            .collect();
        let idx = SearchIndex::build(&fs);
        assert_eq!(idx.search("梁", 3).len(), 3);
        assert_eq!(idx.search("梁", 100).len(), 10);
    }

    #[test]
    fn ties_keep_original_order() {
        let fs: Vec<FormulaSchema> = ["a", "b", "c"]
            .iter()
            .map(|id| formula(id, "梁正截面", "结构", &[]))
            .collect();
        let idx = SearchIndex::build(&fs);
        let found = idx.search("梁", 20);
        let ids: Vec<&str> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "同分保持原顺序（稳定排序）");
    }

    // ---------------- 大小写 / 空白 ----------------

    #[test]
    fn query_is_case_insensitive() {
        let idx = SearchIndex::build(&[formula("f", "HRB400钢筋", "结构", &[])]);
        assert_eq!(idx.search("hrb400", 20).len(), 1);
        assert_eq!(idx.search("HRB400", 20).len(), 1);
    }

    #[test]
    fn query_is_trimmed() {
        let idx = SearchIndex::build(&[formula("f", "梁正截面", "结构", &[])]);
        assert_eq!(idx.search("  梁正  ", 20).len(), 1);
    }

    // ---------------- 建议（轻量结构） ----------------

    #[test]
    fn suggest_returns_lightweight_shape() {
        let idx = SearchIndex::build(&[formula("f1", "梁正截面", "结构", &[])]);
        let s = idx.suggest("梁", 20);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].id, "f1");
        assert_eq!(s[0].result_name, "梁正截面");
        assert_eq!(s[0].domain, "结构");
        assert_eq!(s[0].source_kind, SourceKind::Custom);
    }

    #[test]
    fn suggest_serde_is_camel_case() {
        let s = SearchSuggestion {
            id: "f".into(),
            result_name: "名称".into(),
            domain: "结构".into(),
            source_kind: SourceKind::Standard,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["resultName"], serde_json::json!("名称"));
        assert_eq!(v["sourceKind"], serde_json::json!("STANDARD"));
        assert!(v.get("result_name").is_none(), "不得泄漏 snake_case");
    }

    // ---------------- 索引与源数据一致性 ----------------

    #[test]
    fn index_len_matches_input() {
        let fs: Vec<FormulaSchema> = (0..7)
            .map(|i| formula(&format!("f{i}"), "n", "d", &[]))
            .collect();
        let idx = SearchIndex::build(&fs);
        assert_eq!(idx.len(), 7);
        assert!(!idx.is_empty());
    }

    /// 索引构建后修改原公式**不影响**索引（快照语义）
    #[test]
    fn index_is_a_snapshot() {
        let mut f = formula("f", "梁正截面", "结构", &[]);
        let idx = SearchIndex::build(std::slice::from_ref(&f));
        f.result_name = "完全不同的名字".into();
        assert_eq!(idx.search("梁", 20).len(), 1, "索引应保持构建时的快照");
    }
}
