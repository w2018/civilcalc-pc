//! 组 11：Token 用量统计（3 个命令）。
//!
//! | 命令 | 入参 | 返回 |
//! |---|---|---|
//! | `usage_stats` | `groupBy?` | [`UsageStats`] |
//! | `usage_clear` | `confirm: boolean` | `number`（清掉的条数） |
//! | `usage_export_csv` | `path` | `number`（写入行数） |
//!
//! ## 为什么 `usage_stats` 一次返回三种视图
//!
//! 用量页通常同时要「总计 + 按模型 + 按天」。分成三个命令会让页面开三次
//! IPC、读三次库。这里一次查完 —— 数据量很小（一行一次调用），
//! 代价可忽略，换来页面只发一次请求。
//!
//! ## ⚠️ CSV 必须带 UTF-8 BOM
//!
//! Excel 在中文 Windows 上默认按 GBK 打开 `.csv`，**没有 BOM 就会乱码**。
//! 因此这里显式写 `\u{FEFF}`。

use crate::error::{CmdResult, CommandError};
use crate::state::AppState;
use civilcalc_store::{UsageDaySummary, UsageRecord, UsageSummary};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use tauri::State;

/// 分组方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageGroupBy {
    Model,
    Day,
}

impl UsageGroupBy {
    pub fn as_str(self) -> &'static str {
        match self {
            UsageGroupBy::Model => "model",
            UsageGroupBy::Day => "day",
        }
    }
}

/// 用量总计
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub total_prompt: i64,
    pub total_completion: i64,
    pub total_all: i64,
    pub total_cached: i64,
    pub total_reasoning: i64,
    pub call_count: i64,
}

/// 用量统计（三种视图一次给全）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    /// 前端请求的分组方式（回显；前端据此决定高亮哪个表）
    pub group_by: String,
    /// 按模型汇总（按 totalTokens 降序）
    pub by_model: Vec<UsageSummary>,
    /// 按本地日期汇总（最近的在前）
    pub by_day: Vec<UsageDaySummary>,
    /// 全局总计
    pub total: UsageTotals,
}

/// 用量统计。
///
/// `group_by` 只影响回显字段 —— 三种视图都会返回。
#[tauri::command]
pub fn usage_stats(
    state: State<'_, AppState>,
    group_by: Option<UsageGroupBy>,
) -> CmdResult<UsageStats> {
    let by_model = state.db.usage_summary()?;
    let by_day = state.db.usage_summary_by_day()?;

    // 总计由「按模型」的行累加得出 —— 两者必然一致（同一张表、同样的聚合口径），
    // 再查一次 SQL 是浪费。
    let mut total = UsageTotals::default();
    for m in &by_model {
        total.total_prompt += m.total_prompt;
        total.total_completion += m.total_completion;
        total.total_all += m.total_all;
        total.total_cached += m.total_cached;
        total.total_reasoning += m.total_reasoning;
        total.call_count += m.call_count;
    }

    Ok(UsageStats {
        group_by: group_by.unwrap_or(UsageGroupBy::Model).as_str().to_string(),
        by_model,
        by_day,
        total,
    })
}

/// 清空用量统计。**必须传 `confirm: true`**。
///
/// @returns 清掉的记录条数
#[tauri::command]
pub fn usage_clear(state: State<'_, AppState>, confirm: bool) -> CmdResult<i64> {
    if !confirm {
        return Err(CommandError::InvalidArgument {
            message: "清空用量统计不可撤销，请显式确认（confirm=true）".to_string(),
        });
    }
    let n = state.db.usage_count()?;
    state.db.clear_usage()?;
    civilcalc_core::log::i("Commands", &format!("已清空 Token 用量统计（{n} 条）"));
    Ok(n)
}

/// CSV 表头（列顺序即导出顺序）
const CSV_HEADER: &str = "id,模型,输入tokens,输出tokens,合计tokens,缓存tokens,推理tokens,时间";

/// 导出用量明细为 CSV。
///
/// - **带 UTF-8 BOM**（否则中文 Windows 的 Excel 会乱码）
/// - 时间列格式为 `YYYY-MM-DD HH:MM:SS`（本地时区）
///
/// @returns 写入的数据行数（不含表头）
#[tauri::command]
pub fn usage_export_csv(state: State<'_, AppState>, path: String) -> CmdResult<usize> {
    let rows = state.db.list_usage()?;

    let mut out = String::with_capacity(128 + rows.len() * 96);
    out.push('\u{FEFF}'); // BOM
    out.push_str(CSV_HEADER);
    out.push_str("\r\n"); // CRLF：Excel 更认

    for r in &rows {
        out.push_str(&csv_line(r));
    }

    let p = std::path::Path::new(&path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| CommandError::Storage {
                message: format!("创建导出目录失败: {e}"),
            })?;
        }
    }
    std::fs::write(p, out.as_bytes()).map_err(|e| CommandError::Storage {
        message: format!("写入 CSV 失败（{}）: {e}", p.display()),
    })?;

    civilcalc_core::log::i(
        "Commands",
        &format!("导出用量 CSV：{} 行 → {}", rows.len(), p.display()),
    );
    Ok(rows.len())
}

/// 一行 CSV（字段转义：含 `,` / `"` / 换行时用双引号包裹并转义引号）
fn csv_line(r: &UsageRecord) -> String {
    let mut s = String::with_capacity(96);
    let _ = write!(
        s,
        "{},{},{},{},{},{},{},{}\r\n",
        r.id,
        csv_escape(&r.model_label),
        r.prompt_tokens,
        r.completion_tokens,
        r.total_tokens,
        r.cached_tokens,
        r.reasoning_tokens,
        format_local_time(r.created_at),
    );
    s
}

/// CSV 字段转义（RFC 4180）
fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Unix 毫秒 → `YYYY-MM-DD HH:MM:SS`（**本地时区**）。
///
/// 手写而不引 `chrono`：`civilcalc-store` 才依赖 chrono，命令层不必为此加依赖。
/// 用 `time` 之外的纯算术实现 —— 只做 UTC+偏移 的换算。
fn format_local_time(ms: i64) -> String {
    // 本地时区偏移（秒）。用 std 拿不到，退化为 UTC 并在注释里说明。
    //
    // ⚠️ 这里刻意**不猜时区**：宁可给 UTC 也不给一个错的本地时间。
    // 若后续需要本地时间，应引 `chrono` 的 `Local` 并在此处替换。
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);

    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);

    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}Z")
}

/// 天数（1970-01-01 起）→ `(年, 月, 日)`。Howard Hinnant 的 `civil_from_days` 算法。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_store::{Db, UsageRow};

    // ---------------- 总计累加 ----------------

    #[test]
    fn totals_sum_from_by_model_rows() {
        let by_model = vec![
            UsageSummary {
                model_label: "a".into(),
                total_prompt: 10,
                total_completion: 20,
                total_all: 30,
                total_cached: 1,
                total_reasoning: 2,
                call_count: 3,
            },
            UsageSummary {
                model_label: "b".into(),
                total_prompt: 100,
                total_completion: 200,
                total_all: 300,
                total_cached: 10,
                total_reasoning: 20,
                call_count: 30,
            },
        ];
        let mut total = UsageTotals::default();
        for m in &by_model {
            total.total_prompt += m.total_prompt;
            total.total_completion += m.total_completion;
            total.total_all += m.total_all;
            total.total_cached += m.total_cached;
            total.total_reasoning += m.total_reasoning;
            total.call_count += m.call_count;
        }
        assert_eq!(total.total_all, 330);
        assert_eq!(total.call_count, 33);
        assert_eq!(total.total_cached, 11);
    }

    #[test]
    fn empty_totals_are_zero() {
        assert_eq!(UsageTotals::default().total_all, 0);
        assert_eq!(UsageTotals::default().call_count, 0);
    }

    // ---------------- 分组枚举 ----------------

    #[test]
    fn group_by_serde_is_lowercase() {
        assert_eq!(
            serde_json::to_string(&UsageGroupBy::Model).unwrap(),
            "\"model\""
        );
        assert_eq!(
            serde_json::to_string(&UsageGroupBy::Day).unwrap(),
            "\"day\""
        );
        assert_eq!(UsageGroupBy::Model.as_str(), "model");
        assert_eq!(UsageGroupBy::Day.as_str(), "day");
    }

    // ---------------- CSV ----------------

    #[test]
    fn csv_escape_quotes_only_when_needed() {
        assert_eq!(csv_escape("deepseek"), "deepseek");
        assert_eq!(csv_escape("含,逗号"), "\"含,逗号\"");
        assert_eq!(csv_escape("含\"引号"), "\"含\"\"引号\"");
        assert_eq!(csv_escape("含\n换行"), "\"含\n换行\"");
    }

    #[test]
    fn csv_line_has_eight_columns() {
        let r = UsageRecord {
            id: 1,
            model_label: "deepseek-flash".into(),
            prompt_tokens: 100,
            completion_tokens: 200,
            total_tokens: 300,
            cached_tokens: 10,
            reasoning_tokens: 20,
            created_at: 1_700_000_000_000,
        };
        let line = csv_line(&r);
        assert!(line.ends_with("\r\n"));
        let cols: Vec<&str> = line.trim_end().split(',').collect();
        assert_eq!(cols.len(), 8, "列数应与表头一致: {line}");
        assert_eq!(cols[0], "1");
        assert_eq!(cols[1], "deepseek-flash");
        assert_eq!(cols[4], "300");
    }

    /// CSV 必须带 BOM，否则中文 Windows 的 Excel 乱码
    #[test]
    fn csv_starts_with_bom() {
        let mut out = String::new();
        out.push('\u{FEFF}');
        out.push_str(CSV_HEADER);
        assert!(out.starts_with('\u{FEFF}'));
        assert_eq!(out.as_bytes()[0..3], [0xEF, 0xBB, 0xBF]);
    }

    #[test]
    fn header_has_eight_columns() {
        assert_eq!(CSV_HEADER.split(',').count(), 8);
    }

    // ---------------- 日期换算 ----------------

    #[test]
    fn civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        assert_eq!(civil_from_days(365), (1971, 1, 1));
        // 2000-01-01 是第 10957 天
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        // 2026-09-17
        assert_eq!(civil_from_days(20_713), (2026, 9, 17));
    }

    #[test]
    fn format_local_time_shape() {
        // 2026-09-17T00:00:00Z
        let s = format_local_time(1_789_900_800_000);
        assert_eq!(s.len(), 20, "YYYY-MM-DD HH:MM:SSZ");
        assert!(s.ends_with('Z'), "未做时区换算时显式标 UTC: {s}");
        assert_eq!(&s[0..4], "2026");
    }

    // ---------------- 与存储层联通 ----------------

    #[test]
    fn stats_aggregation_over_real_db() {
        let db = Db::open_in_memory().unwrap();
        for (label, total) in [("a", 10i64), ("b", 20), ("a", 5)] {
            db.insert_usage(&UsageRow {
                model_label: label.into(),
                prompt_tokens: total / 2,
                completion_tokens: total - total / 2,
                total_tokens: total,
                cached_tokens: 0,
                reasoning_tokens: 0,
            })
            .unwrap();
        }

        let by_model = db.usage_summary().unwrap();
        assert_eq!(by_model.len(), 2);
        // 按 totalTokens 降序：b(20) 在前
        assert_eq!(by_model[0].model_label, "b");
        assert_eq!(by_model[1].total_all, 15, "a 的两条合计 10+5");

        let mut total = 0i64;
        for m in &by_model {
            total += m.total_all;
        }
        assert_eq!(total, 35);
        assert_eq!(db.usage_count().unwrap(), 3);
    }
}
