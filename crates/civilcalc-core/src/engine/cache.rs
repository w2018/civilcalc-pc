//! 表达式编译缓存（LRU，容量 256）。
//!
//! P2-2 是 **PC 端新增的优化**（源项目 `FormulaEngineImpl` 没有缓存）。
//!
//! ## 为什么值得缓存
//!
//! 真正的热路径不是"同一表达式被反复编译"，而是
//! **表达式不变、只改输入值** 的实时预览场景：
//! 用户在参数表里改一个数 → 300ms 防抖后重新求值 → 表达式一模一样。
//! 没有缓存时每次都要重跑「词法分析 → 语法分析」。
//!
//! ## ⚠️ 缓存键不能只是表达式字符串
//!
//! `docs/07` 的 P2-2 写的是「key = 表达式字符串」，但那是**不够的**：
//! 词法分析会把**常量值烤进 token**（[`RpnToken::Constant`] 带 `value`），
//! 因此同一个表达式在不同常量表下会编译出**不同的 RPN**：
//!
//! ```
//! # use std::collections::HashMap;
//! # use civilcalc_core::engine::{compile, types::RpnToken};
//! let mut c1 = HashMap::new(); c1.insert("k".to_string(), 1.0);
//! let mut c2 = HashMap::new(); c2.insert("k".to_string(), 2.0);
//!
//! let e1 = compile("k*2", &c1);
//! let e2 = compile("k*2", &c2);
//! assert_ne!(e1, e2, "同一表达式、不同常量 → 不同 RPN");
//! ```
//!
//! 所以键是 **`(表达式, 常量表)`** —— 常量表按 key 排序后拼进键里。
//! 只用表达式做键会让「弹性模量 = 206000」的公式命中
//! 「弹性模量 = 100」的缓存，**静默算错**。
//!
//! ## 错误也缓存
//!
//! 用户在编辑框里打字时，中间态（如 `a +`）会反复编译失败。
//! 缓存错误可以避免每次都重跑一遍注定失败的词法/语法分析。
//!
//! ## 线程安全
//!
//! Tauri 命令可能在不同线程上跑，因此用 `Mutex`。
//! 锁的临界区只有 map 操作，**不含编译本身** ——
//! 编译在锁外做，避免长表达式阻塞其他线程。

use crate::engine::types::{CompileResult, CompiledExpr};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// 默认容量（对齐 `docs/07` P2-2 的「上限 256」）
pub const DEFAULT_CAPACITY: usize = 256;

/// 键里用来分隔「表达式」与「常量表」的字符。
///
/// 用 `\u{1}`（SOH）而不是 `|` / `;` —— 前者不可能出现在用户输入里，
/// 后者可能（表达式里可以写 `a|b` 吗？虽然不支持，但不要赌）。
const KEY_SEP: char = '\u{1}';

/// 缓存里的一条编译结果。
#[derive(Debug, Clone)]
pub enum CachedCompile {
    /// 成功。用 `Arc` 让缓存命中时**零克隆**
    Ok(Arc<CompiledExpr>),
    /// 失败（位置 + 原因）
    Err {
        position: Option<usize>,
        reason: String,
    },
}

impl CachedCompile {
    /// 转成对外的 [`CompileResult`]（`Ok` 分支会**克隆** RPN）
    pub fn to_result(&self) -> CompileResult {
        match self {
            CachedCompile::Ok(e) => CompileResult::Ok {
                expr: (**e).clone(),
            },
            CachedCompile::Err { position, reason } => CompileResult::Error {
                position: *position,
                reason: reason.clone(),
            },
        }
    }

    /// 取编译好的表达式（**零克隆**，热路径用）
    pub fn as_expr(&self) -> Option<&Arc<CompiledExpr>> {
        match self {
            CachedCompile::Ok(e) => Some(e),
            CachedCompile::Err { .. } => None,
        }
    }

    /// 是否成功
    pub fn is_ok(&self) -> bool {
        matches!(self, CachedCompile::Ok(_))
    }
}

/// 缓存统计（排障用）
///
/// 可直接序列化给前端（"关于"页的排障区块）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub len: usize,
    pub capacity: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

impl CacheStats {
    /// 命中率（无查询时返回 0，不返回 NaN）
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    map: HashMap<String, (CachedCompile, u64)>,
    /// 逻辑时钟，每次访问 +1；淘汰时取最小者
    tick: u64,
}

/// LRU 编译缓存。
///
/// ## 淘汰策略
///
/// 容量满时淘汰**最久未使用**的一条。因为容量只有 256，
/// 线性扫描找最小值（O(256)）比维护双向链表简单得多，且常数极小。
pub struct CompileCache {
    inner: Mutex<Inner>,
    capacity: usize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl std::fmt::Debug for CompileCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompileCache")
            .field("stats", &self.stats())
            .finish()
    }
}

impl Default for CompileCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl CompileCache {
    /// 容量为 0 时**不缓存**（`get_or_compile` 每次都直接编译）—— 便于对照测试
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            capacity,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// 查缓存，未命中则用 `compile_fn` 编译并存入。
    ///
    /// ⚠️ **编译在锁外执行**（先查、放锁、编译、再加锁存）——
    /// 否则一个长表达式会阻塞所有其他线程。
    /// 代价是并发下可能重复编译同一个表达式（幂等，可接受）。
    pub fn get_or_compile<F>(&self, key: &str, compile_fn: F) -> CachedCompile
    where
        F: FnOnce() -> CachedCompile,
    {
        if self.capacity == 0 {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return compile_fn();
        }

        // 1. 查（持锁）
        if let Ok(mut g) = self.inner.lock() {
            g.tick += 1;
            let tick = g.tick;
            if let Some((v, last)) = g.map.get_mut(key) {
                *last = tick;
                self.hits.fetch_add(1, Ordering::Relaxed);
                return v.clone();
            }
        }
        self.misses.fetch_add(1, Ordering::Relaxed);

        // 2. 编译（**不持锁**）
        let fresh = compile_fn();

        // 3. 存（持锁）
        if let Ok(mut g) = self.inner.lock() {
            g.tick += 1;
            let tick = g.tick;

            // 已在别处存过 → 用已有的（保持一致性）
            if let Some((v, last)) = g.map.get_mut(key) {
                *last = tick;
                return v.clone();
            }

            if g.map.len() >= self.capacity {
                if let Some(k) = g
                    .map
                    .iter()
                    .min_by_key(|(_, (_, last))| *last)
                    .map(|(k, _)| k.clone())
                {
                    g.map.remove(&k);
                    self.evictions.fetch_add(1, Ordering::Relaxed);
                }
            }
            g.map.insert(key.to_string(), (fresh.clone(), tick));
        }

        fresh
    }

    /// 当前统计
    pub fn stats(&self) -> CacheStats {
        let len = self.inner.lock().map(|g| g.map.len()).unwrap_or(0);
        CacheStats {
            len,
            capacity: self.capacity,
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }

    /// 清空（**统计也归零** —— 便于测试与"重置软件"）
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.map.clear();
            g.tick = 0;
        }
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
    }

    /// 容量
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// 构造缓存键：`表达式 \u{1} 常量k=值;常量k2=值2;`（常量按 key 排序）。
///
/// 常量用 `{:?}`（f64 的最短往返表示）而不是 `{}`：
/// - 确定性：同样的 f64 永远产生同样的串
/// - 区分度：`1.0` / `1` / `1e0` 都是同一个值，但 `{:?}` 统一输出 `1.0`
///
/// 常量表为空时直接返回表达式本身（避免无谓分配）。
pub fn cache_key(expr: &str, constants: &HashMap<String, f64>) -> String {
    if constants.is_empty() {
        // 空常量时**也要带分隔符**：否则 `"a" + {b:1}`（键 `"a\u{1}b=1.0;"`）
        // 会与 `"a\u{1}b=1.0;" + {}`（键就是表达式本身）撞键。
        return format!("{expr}{KEY_SEP}");
    }

    let mut items: Vec<(&str, f64)> = constants.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    items.sort_unstable_by(|a, b| a.0.cmp(b.0));

    let mut s = String::with_capacity(expr.len() + items.len() * 24 + 1);
    s.push_str(expr);
    s.push(KEY_SEP);
    for (k, v) in items {
        s.push_str(k);
        s.push('=');
        s.push_str(&format!("{v:?}"));
        s.push(';');
    }
    s
}

// =============================================================================
// 全局缓存
// =============================================================================

/// 全局编译缓存（所有命令共用）。
///
/// 为什么用全局而不是每处 `new` 一个：缓存的价值在于**跨命令、跨请求**复用 ——
/// 用户在参数表里连改 5 个值会触发 5 次求值，表达式却始终不变。
static GLOBAL: std::sync::LazyLock<CompileCache> =
    std::sync::LazyLock::new(|| CompileCache::new(DEFAULT_CAPACITY));

/// 取全局缓存（排障 / 测试用）
pub fn global_cache() -> &'static CompileCache {
    &GLOBAL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{compile, types::RpnToken};

    fn constants(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn compile_to_cached(expr: &str, c: &HashMap<String, f64>) -> CachedCompile {
        match compile(expr, c) {
            CompileResult::Ok { expr } => CachedCompile::Ok(Arc::new(expr)),
            CompileResult::Error { position, reason } => CachedCompile::Err { position, reason },
        }
    }

    // ---------------- 键构造 ----------------

    #[test]
    fn key_is_expression_plus_separator_when_no_constants() {
        let k = cache_key("a+b", &HashMap::new());
        assert!(k.starts_with("a+b"));
        assert_eq!(k, format!("a+b{KEY_SEP}"));
    }

    #[test]
    fn key_includes_sorted_constants() {
        let c1 = constants(&[("b", 2.0), ("a", 1.0)]);
        let c2 = constants(&[("a", 1.0), ("b", 2.0)]);
        assert_eq!(cache_key("x", &c1), cache_key("x", &c2), "常量顺序不应影响键");
    }

    /// **关键回归**：同一表达式 + 不同常量 → **不同键**
    #[test]
    fn key_differs_for_different_constant_values() {
        let k1 = cache_key("k*2", &constants(&[("k", 1.0)]));
        let k2 = cache_key("k*2", &constants(&[("k", 2.0)]));
        assert_ne!(k1, k2, "常量值不同必须产生不同键，否则会静默算错");
    }

    #[test]
    fn key_differs_for_different_constant_names() {
        let k1 = cache_key("x*2", &constants(&[("x", 1.0)]));
        let k2 = cache_key("x*2", &constants(&[("y", 1.0)]));
        assert_ne!(k1, k2);
    }

    /// 分隔符要保证「表达式与常量拼接」无歧义。
    ///
    /// 构造一个**故意含分隔符**的表达式，验证它不会与
    /// 「短表达式 + 常量」撞键。
    #[test]
    fn key_separates_expression_from_constants() {
        let with_const = cache_key("a", &constants(&[("b", 1.0)])); // "a\u{1}b=1.0;"
        let as_expr = cache_key(&with_const, &HashMap::new()); // 把上面那个键当成表达式
        assert_ne!(with_const, as_expr, "分隔符要保证拼接无歧义");
    }

    /// **关键回归**：不同常量确实编译出不同 RPN（这就是必须把常量放进键的原因）
    #[test]
    fn different_constants_yield_different_rpn() {
        let e1 = compile("k*2", &constants(&[("k", 1.0)]));
        let e2 = compile("k*2", &constants(&[("k", 2.0)]));
        assert_ne!(e1, e2);

        // 取出 RPN 确认常量值真的被烤进去了
        if let (CompileResult::Ok { expr: a }, CompileResult::Ok { expr: b }) = (&e1, &e2) {
            let has = |e: &CompiledExpr, v: f64| {
                e.rpn.iter().any(|t| matches!(t, RpnToken::Constant { value, .. } if (*value - v).abs() < 1e-12))
            };
            assert!(has(a, 1.0), "k=1 应烤进 1.0");
            assert!(has(b, 2.0), "k=2 应烤进 2.0");
        } else {
            panic!("应编译成功");
        }
    }

    // ---------------- 命中 / 未命中 ----------------

    #[test]
    fn first_call_misses_second_hits() {
        let cache = CompileCache::new(16);
        let c = constants(&[("k", 1.0)]);
        let key = cache_key("k*2", &c);

        let a = cache.get_or_compile(&key, || compile_to_cached("k*2", &c));
        assert!(a.is_ok());
        assert_eq!(cache.stats().hits, 0);
        assert_eq!(cache.stats().misses, 1);

        let b = cache.get_or_compile(&key, || compile_to_cached("k*2", &c));
        assert!(b.is_ok());
        assert_eq!(cache.stats().hits, 1, "第二次应命中");
        assert_eq!(cache.stats().misses, 1);
    }

    /// 命中时**不调用** compile_fn（这是缓存的意义）
    #[test]
    fn hit_does_not_recompile() {
        let cache = CompileCache::new(4);
        let calls = std::cell::Cell::new(0);

        let _ = cache.get_or_compile("a", || {
            calls.set(calls.get() + 1);
            compile_to_cached("a+b", &HashMap::new())
        });
        let _ = cache.get_or_compile("a", || {
            calls.set(calls.get() + 1);
            compile_to_cached("a+b", &HashMap::new())
        });

        assert_eq!(calls.get(), 1, "第二次不该再编译");
    }

    #[test]
    fn zero_capacity_never_caches() {
        let cache = CompileCache::new(0);
        for _ in 0..3 {
            let _ = cache.get_or_compile("a", || compile_to_cached("a+b", &HashMap::new()));
        }
        assert_eq!(cache.stats().len, 0);
        assert_eq!(cache.stats().hits, 0);
        assert_eq!(cache.stats().misses, 3);
    }

    /// 编译错误也缓存（编辑框打字中间态会反复失败）
    #[test]
    fn errors_are_cached_too() {
        let cache = CompileCache::new(4);
        let calls = std::cell::Cell::new(0);

        for _ in 0..2 {
            let r = cache.get_or_compile("bad", || {
                calls.set(calls.get() + 1);
                // 用确定编译失败的表达式：括号未闭合
                compile_to_cached("a + (b", &HashMap::new())
            });
            assert!(!r.is_ok(), "a + (b 应编译失败（括号未闭合）");
        }
        assert_eq!(calls.get(), 1, "第二次应命中缓存的错误");
        assert_eq!(cache.stats().hits, 1);
    }

    // ---------------- LRU 淘汰 ----------------

    #[test]
    fn evicts_least_recently_used() {
        let cache = CompileCache::new(2);
        let mk = |s: &str| {
            let owned = s.to_string();
            move || compile_to_cached(&owned, &HashMap::new())
        };

        cache.get_or_compile("a", mk("a"));
        cache.get_or_compile("b", mk("b"));
        // 访问 a，使 b 变成最久未使用
        cache.get_or_compile("a", mk("a"));

        // 插入 c → 应淘汰 b
        cache.get_or_compile("c", mk("c"));

        assert_eq!(cache.stats().len, 2);
        assert_eq!(cache.stats().evictions, 1);

        // b 应未命中，a 应命中
        let before = cache.stats();
        cache.get_or_compile("a", mk("a"));
        assert_eq!(cache.stats().hits, before.hits + 1, "a 应仍在缓存里");

        let before = cache.stats();
        cache.get_or_compile("b", mk("b"));
        assert_eq!(cache.stats().misses, before.misses + 1, "b 应已被淘汰");
    }

    #[test]
    fn capacity_is_respected() {
        let cache = CompileCache::new(8);
        for i in 0..50 {
            let k = format!("e{i}");
            cache.get_or_compile(&k, || compile_to_cached(&k, &HashMap::new()));
        }
        let s = cache.stats();
        assert_eq!(s.len, 8, "长度不得超过容量");
        assert_eq!(s.capacity, 8);
        assert_eq!(s.evictions, 42, "50 - 8");
    }

    // ---------------- 统计 / 清空 ----------------

    #[test]
    fn hit_rate_is_zero_when_no_queries() {
        let cache = CompileCache::new(4);
        assert_eq!(cache.stats().hit_rate(), 0.0, "无查询时返回 0 而非 NaN");
    }

    #[test]
    fn hit_rate_computes() {
        let cache = CompileCache::new(4);
        cache.get_or_compile("a", || compile_to_cached("a", &HashMap::new()));
        cache.get_or_compile("a", || compile_to_cached("a", &HashMap::new()));
        cache.get_or_compile("a", || compile_to_cached("a", &HashMap::new()));
        assert!((cache.stats().hit_rate() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn clear_resets_everything() {
        let cache = CompileCache::new(4);
        cache.get_or_compile("a", || compile_to_cached("a", &HashMap::new()));
        cache.get_or_compile("a", || compile_to_cached("a", &HashMap::new()));
        assert!(cache.stats().hits > 0);

        cache.clear();
        let s = cache.stats();
        assert_eq!(s.len, 0);
        assert_eq!(s.hits, 0);
        assert_eq!(s.misses, 0);
        assert_eq!(s.evictions, 0);
    }

    // ---------------- 与对外类型的转换 ----------------

    #[test]
    fn cached_ok_converts_back_to_result() {
        let c = compile_to_cached("a+b", &HashMap::new());
        match c.to_result() {
            CompileResult::Ok { expr } => assert!(!expr.rpn.is_empty()),
            _ => panic!("应成功"),
        }
        assert!(c.as_expr().is_some());
    }

    #[test]
    fn cached_err_converts_back_to_result() {
        let c = compile_to_cached("a + (b", &HashMap::new());
        match c.to_result() {
            CompileResult::Error { reason, .. } => assert!(!reason.is_empty()),
            _ => panic!("应失败"),
        }
        assert!(c.as_expr().is_none());
    }

    #[test]
    fn default_capacity_matches_spec() {
        assert_eq!(CompileCache::default().capacity(), 256);
        assert_eq!(global_cache().capacity(), 256);
    }

    // ---------------- 全局缓存 ----------------

    /// ⚠️ 本测试**不调 `clear()`** —— 全局缓存被所有测试共享，
    /// 在并发测试里清空会干扰其他测试的统计断言。
    /// 这里只验证"全局缓存存在且容量正确"，命中行为由隔离实例的测试覆盖。
    #[test]
    fn global_cache_exists_with_default_capacity() {
        assert_eq!(global_cache().capacity(), DEFAULT_CAPACITY);
    }
}

