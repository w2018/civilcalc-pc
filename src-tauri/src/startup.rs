//! 启动时序（对齐源项目 `CivilCalcApp.onCreate`）。
//!
//! ## 源项目顺序（`CivilCalcApp.kt`）
//!
//! 1. `CivilLog.install { ... }` —— 把纯 JVM 的日志门面桥接到 Logcat（BUG-24）
//! 2. `CivilCalcDatabase.getInstance(this)` —— 建库 + 迁移（1→2→3）
//! 3. **异步**：读 `assets/builtin_formulas.json` → `BuiltinFormulaLoader.load`
//!    → 库为空则播种 → 对 `schemaVersion < 3` 的公式做 Excel 字段迁移
//! 4. 任一步抛异常 → `IllegalStateException("内置公式来源校验失败: …")` → **拒绝启动**
//!
//! ## PC 端顺序（`docs/07` P1-6）
//!
//! | # | 步骤 | 状态 |
//! |---|---|---|
//! | 1 | `DesktopPaths` 解析（`app_data_dir` / `app_cache_dir`） | ✅ 在 `lib.rs` |
//! | 2 | `Db::open()` —— 建目录 + 建表 + `user_version` 迁移 | ✅ 在 `lib.rs` |
//! | 3 | 内置库校验 → 播种 | ✅ 本模块 [`init_store`] |
//! | 4 | Excel 字段迁移（`schemaVersion < 3`） | ⏳ P3-1（`civilcalc-core/excel`） |
//! | 5 | 搜索索引重建 | ✅ 在 `init_store()` 末尾 |
//!
//! ## 与源项目的两处刻意差异
//!
//! - **同步而非异步**：源项目用 IO 协程异步播种，异常在协程里抛出（Android 上表现为崩溃）。
//!   PC 端**在 `setup()` 里同步做完**再开窗口 —— 语义更明确（"启动失败就是启动失败"），
//!   也避免"窗口已出现但数据还没准备好"的中间态。
//! - **可测试**：[`init_store`] 只依赖 [`Db`]，不碰 `tauri`，
//!   因此能用内存库直接跑单测。

use civilcalc_core::search::SearchIndex;
use civilcalc_core::source::builtin_loader;
use civilcalc_store::Db;

/// 启动结果（供日志与「关于」页展示）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupReport {
    /// 内置库总条数（= 校验通过 + 跳过）
    pub builtin_total: usize,
    /// 校验通过的条数
    pub builtin_valid: usize,
    /// 校验被跳过的条目（格式 `"{id}: {原因}"`）
    pub builtin_skipped: Vec<String>,
    /// 本次实际播种的行数（库非空时为 0）
    pub seeded: usize,
}

impl StartupReport {
    /// 是否执行了首次播种
    pub fn did_seed(&self) -> bool {
        self.seeded > 0
    }
}

/// 内置库校验 + 首次播种（第 3 步）。
///
/// ## 失败即拒绝启动
///
/// 返回 `Err` 时调用方**必须中止启动**，不要吞掉 —— 这是源项目
/// "内置公式来源校验失败就拒绝启动"的工程安全底线。
///
/// 两类失败：
/// - `builtin_loader::load_embedded()` 返回 `Err`（JSON 坏了 / **全部条目校验失败**）
/// - 播种时数据库报错
///
/// ## 部分条目被跳过怎么办
///
/// 源项目对"部分失败"是**容忍**的（记 WARN 日志，继续跑）；
/// 只有**全部失败**才拒绝启动。这里保持一致。
pub fn init_store(db: &Db) -> Result<(StartupReport, SearchIndex), String> {
    let loaded = builtin_loader::load_embedded()
        .map_err(|e| format!("内置公式来源校验失败: {e}"))?;

    let total = loaded.valid.len() + loaded.skipped.len();

    if !loaded.skipped.is_empty() {
        civilcalc_core::log::w(
            "Startup",
            &format!(
                "{} 条内置公式未通过来源校验，已跳过（共 {} 条）",
                loaded.skipped.len(),
                total
            ),
            None,
        );
        for item in &loaded.skipped {
            civilcalc_core::log::w("Startup", &format!("  跳过: {item}"), None);
        }
    }

    // 仅在 user_formulas 为空时播种（幂等）
    let seeded = db
        .seed_builtins_if_empty(&loaded.valid)
        .map_err(|e| format!("播种内置公式失败: {e}"))?;

    civilcalc_core::log::i(
        "Startup",
        &format!(
            "内置库就绪：校验通过 {} / 跳过 {}；本次播种 {seeded} 条",
            loaded.valid.len(),
            loaded.skipped.len()
        ),
    );

    // 4. 搜索索引（P2-4）
    //    必须在播种**之后**建 —— 否则首次启动时索引是空的，搜不到内置公式。
    let formulas = db
        .list_formulas()
        .map_err(|e| format!("读取公式列表失败（建索引前）: {e}"))?;
    let index = SearchIndex::build(&formulas);
    civilcalc_core::log::i(
        "Startup",
        &format!("检索索引就绪：{} 条", index.len()),
    );

    Ok((
        StartupReport {
            builtin_total: total,
            builtin_valid: loaded.valid.len(),
            builtin_skipped: loaded.skipped,
            seeded,
        },
        index,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Db {
        Db::open_in_memory().expect("内存库应可打开")
    }

    /// 首次启动：空库 → 播种 55 条内置公式
    #[test]
    fn first_launch_seeds_all_builtins() {
        let db = mem_db();
        let (r, index) = init_store(&db).expect("内置库应能校验通过");
        assert_eq!(index.len(), 55, "索引应含 55 条");

        assert_eq!(r.builtin_valid, 55, "55 条内置公式应全部通过校验");
        assert!(r.builtin_skipped.is_empty(), "不应有跳过条目");
        assert_eq!(r.builtin_total, 55);
        assert_eq!(r.seeded, 55);
        assert!(r.did_seed());

        // 库里确实有 55 条
        assert_eq!(db.list_formulas().unwrap().len(), 55);
    }

    /// 第二次启动：库非空 → **不重复播种**（幂等）
    #[test]
    fn second_launch_does_not_reseed() {
        let db = mem_db();
        let (first, _) = init_store(&db).unwrap();
        assert_eq!(first.seeded, 55);

        let (second, _) = init_store(&db).expect("第二次也应成功");
        assert_eq!(second.seeded, 0, "库非空时不得重复播种");
        assert!(!second.did_seed());
        assert_eq!(db.list_formulas().unwrap().len(), 55);
    }

    /// 用户已有自己的公式时，**不得**被内置库覆盖或清空
    #[test]
    fn existing_user_data_is_preserved() {
        let db = mem_db();
        // 模拟用户先存了一条自定义公式
        let mut s: civilcalc_core::schema::FormulaSchema = serde_json::from_str(
            r#"{"id":"usr:mine","resultName":"我的","resultSymbol":"y","expression":"1+1",
                "variables":[],"constants":{},"domain":"通用",
                "source":{"kind":"CUSTOM"},"schemaVersion":3}"#,
        )
        .unwrap();
        s.updated_at = 9999;
        db.upsert_formula(&s).unwrap();

        let (r, _) = init_store(&db).expect("启动应成功");
        assert_eq!(r.seeded, 0, "库非空，不应播种内置库");

        // 用户的公式还在
        assert!(db.formula_exists("usr:mine").unwrap());
        assert_eq!(db.list_formulas().unwrap().len(), 1);
    }

    /// 播种后每条内置公式都应能读回且 `schemaVersion = 3`
    #[test]
    fn seeded_builtins_are_readable() {
        let db = mem_db();
        init_store(&db).unwrap();

        let all = db.list_formulas().unwrap();
        assert_eq!(all.len(), 55);
        for f in &all {
            assert_eq!(
                f.schema_version,
                civilcalc_core::schema::CURRENT_SCHEMA_VERSION,
                "{} 的 schemaVersion 应为当前版本",
                f.id
            );
            assert!(!f.expression.is_empty(), "{} 的表达式不应为空", f.id);
        }
    }

    /// `StartupReport::did_seed` 语义
    #[test]
    fn report_did_seed_flag() {
        let no = StartupReport {
            builtin_total: 55,
            builtin_valid: 55,
            builtin_skipped: vec![],
            seeded: 0,
        };
        assert!(!no.did_seed());

        let yes = StartupReport { seeded: 1, ..no };
        assert!(yes.did_seed());
    }
}
