//! 轻量日志门面（替代源项目 `CivilLog`）。
//!
//! 源：`civilcalc-android-v2/core/log/CivilLog.kt`
//!
//! ## 为什么需要门面（源项目 BUG-24）
//!
//! 源项目 `:core` 是**纯 JVM 模块**，不能直接依赖 `android.util.Log`，
//! 因此自建 `CivilLog` 门面，由 Application 启动时安装 Logcat sink。
//!
//! PC 端同理：本 crate **不依赖 `tauri`**（ADR-002），因此不能直接写文件日志。
//! 这里用 [`tracing`] 作为默认 sink —— `src-tauri` 的 `tauri-plugin-log` 会捕获它，
//! 决定输出到控制台还是日志文件。
//!
//! ## 两条硬约束（继承源项目）
//!
//! 1. **门面自身绝不抛异常** —— 日志失败不能影响业务
//! 2. **严禁记录敏感信息** —— API Key / WebDAV 密码 / LLM 原始响应中的敏感字段
//!    （源项目 `LlmError` 把 `errorDetail`/`rawResponse` 与用户可见文案严格分离）
//!
//! ## 用法
//!
//! ```ignore
//! civilcalc_core::log::w("ExcelMigrator", "迁移失败，schema 保持原样", Some(&e.to_string()));
//! // 或宏形式：
//! log_w!("ExcelMigrator", "迁移失败", &e.to_string());
//! ```

use once_cell::sync::Lazy;
use std::sync::RwLock;

/// 日志级别（对齐源项目 `CivilLog.Level`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

/// 自定义 sink：`(level, tag, message, error)`
pub type Sink = Box<dyn Fn(Level, &str, &str, Option<&str>) + Send + Sync>;

/// 默认 sink：经 `tracing` 输出（由 `src-tauri` 的 `tauri-plugin-log` 决定去向）
fn default_sink(level: Level, tag: &str, message: &str, error: Option<&str>) {
    let err_suffix = error.map(|e| format!(" ({e})")).unwrap_or_default();
    match level {
        Level::Debug => tracing::debug!(target: "civilcalc", "[{tag}] {message}{err_suffix}"),
        Level::Info => tracing::info!(target: "civilcalc", "[{tag}] {message}{err_suffix}"),
        Level::Warn => tracing::warn!(target: "civilcalc", "[{tag}] {message}{err_suffix}"),
        Level::Error => tracing::error!(target: "civilcalc", "[{tag}] {message}{err_suffix}"),
    }
}

static SINK: Lazy<RwLock<Sink>> = Lazy::new(|| RwLock::new(Box::new(default_sink)));

/// 安装自定义 sink。
///
/// `src-tauri` 可用它把日志桥接到 `tauri-plugin-log` 或前端
/// （对应 Android 端安装 Logcat sink 的能力）。
pub fn install(custom: Sink) {
    if let Ok(mut guard) = SINK.write() {
        *guard = custom;
    }
}

/// 恢复默认 sink（测试用）
pub fn reset_sink() {
    if let Ok(mut guard) = SINK.write() {
        *guard = Box::new(default_sink);
    }
}

/// 分发日志。
///
/// **绝不 panic**：sink 内部异常或锁中毒都被吞掉（源项目注释：
/// "日志门面自身绝不抛异常"）。
fn dispatch(level: Level, tag: &str, message: &str, error: Option<&str>) {
    // ① 先转发给 `log` crate
    forward_to_log_crate(level, tag, message, error);

    // ② 再走自研 sink（源项目 CivilLog 的桥接）
    let Ok(guard) = SINK.read() else {
        // 锁中毒：放弃本次日志，不影响业务
        return;
    };
    // 源项目用 try/catch 包裹 sink 调用；此处用 catch_unwind 保持同一契约
    let sink = &*guard;
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sink(level, tag, message, error);
    }));
}

/// 把一条记录转发给 `log` crate（即 `tauri-plugin-log` 的接收口）。
///
/// ## 为什么必须有这一步
///
/// `tauri-plugin-log` 只捕获**经过 `log` crate** 的记录。
/// 本模块是自研门面（对齐源项目 `CivilLog`），只往自己的 sink 写 ——
/// 结果就是 `tauri.conf.json` 里配的 `stdout` / `logDir` 目标
/// **什么都收不到**：日志目录里躺着一个**永远 0 字节**的 `.log` 文件。
///
/// 这在排障时是致命的 —— 启动阶段 panic（release 无控制台）时，
/// 唯一能留下线索的地方就是那个文件，而它是空的。
///
/// `target` 用 `tag`（`Startup` / `Commands` / …），便于按模块过滤。
fn forward_to_log_crate(level: Level, tag: &str, message: &str, error: Option<&str>) {
    let lvl = match level {
        Level::Debug => log::Level::Debug,
        Level::Info => log::Level::Info,
        Level::Warn => log::Level::Warn,
        Level::Error => log::Level::Error,
    };
    match error {
        Some(e) => log::log!(target: tag, lvl, "{message} | {e}"),
        None => log::log!(target: tag, lvl, "{message}"),
    }
}

// =============================================================================
// 函数式 API（对齐源项目 CivilLog.i/w/e）
// =============================================================================

/// DEBUG 级日志
pub fn d(tag: &str, message: &str) {
    dispatch(Level::Debug, tag, message, None);
}

/// INFO 级日志
pub fn i(tag: &str, message: &str) {
    dispatch(Level::Info, tag, message, None);
}

/// WARN 级日志（可带错误描述）
pub fn w(tag: &str, message: &str, error: Option<&str>) {
    dispatch(Level::Warn, tag, message, error);
}

/// ERROR 级日志（可带错误描述）
pub fn e(tag: &str, message: &str, error: Option<&str>) {
    dispatch(Level::Error, tag, message, error);
}

/// 格式化一行（默认格式，供测试断言）
///
/// 形如 `[CivilCalc/WARN] ExcelMigrator: 迁移失败 (boom)`
pub fn format_line(level: Level, tag: &str, message: &str, error: Option<&str>) -> String {
    let err_suffix = error.map(|e| format!(" ({e})")).unwrap_or_default();
    format!("[CivilCalc/{}] {tag}: {message}{err_suffix}", level.as_str())
}

// =============================================================================
// 宏（便于在业务代码中调用）
// =============================================================================

/// `log_d!("tag", "message")`
#[macro_export]
macro_rules! log_d {
    ($tag:expr, $msg:expr) => {
        $crate::log::d($tag, $msg)
    };
}

/// `log_i!("tag", "message")`
#[macro_export]
macro_rules! log_i {
    ($tag:expr, $msg:expr) => {
        $crate::log::i($tag, $msg)
    };
}

/// `log_w!("tag", "message")` 或 `log_w!("tag", "message", err)`
#[macro_export]
macro_rules! log_w {
    ($tag:expr, $msg:expr) => {
        $crate::log::w($tag, $msg, None)
    };
    ($tag:expr, $msg:expr, $err:expr) => {
        $crate::log::w($tag, $msg, Some($err))
    };
}

/// `log_e!("tag", "message")` 或 `log_e!("tag", "message", err)`
#[macro_export]
macro_rules! log_e {
    ($tag:expr, $msg:expr) => {
        $crate::log::e($tag, $msg, None)
    };
    ($tag:expr, $msg:expr, $err:expr) => {
        $crate::log::e($tag, $msg, Some($err))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// 测试期间收集日志的 sink。
    ///
    /// ⚠️ `SINK` 是进程级全局，因此**日志相关测试必须串行**
    /// （用 `TEST_LOCK` 保证；否则并行测试会互相覆盖 sink）。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn install_collector() -> Arc<Mutex<Vec<String>>> {
        let collected: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let c = collected.clone();
        install(Box::new(move |level, tag, message, error| {
            if let Ok(mut g) = c.lock() {
                g.push(format_line(level, tag, message, error));
            }
        }));
        collected
    }

    #[test]
    fn default_format_matches_source() {
        assert_eq!(
            format_line(Level::Warn, "ExcelMigrator", "迁移失败", None),
            "[CivilCalc/WARN] ExcelMigrator: 迁移失败"
        );
        assert_eq!(
            format_line(Level::Error, "T", "boom", Some("io error")),
            "[CivilCalc/ERROR] T: boom (io error)"
        );
    }

    #[test]
    fn levels_have_expected_names() {
        assert_eq!(Level::Debug.as_str(), "DEBUG");
        assert_eq!(Level::Info.as_str(), "INFO");
        assert_eq!(Level::Warn.as_str(), "WARN");
        assert_eq!(Level::Error.as_str(), "ERROR");
    }

    #[test]
    fn custom_sink_receives_all_levels() {
        let _guard = TEST_LOCK.lock().unwrap();
        let collected = install_collector();

        d("T", "d-msg");
        i("T", "i-msg");
        w("T", "w-msg", None);
        e("T", "e-msg", Some("cause"));

        let lines = collected.lock().unwrap().clone();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "[CivilCalc/DEBUG] T: d-msg");
        assert_eq!(lines[1], "[CivilCalc/INFO] T: i-msg");
        assert_eq!(lines[2], "[CivilCalc/WARN] T: w-msg");
        assert_eq!(lines[3], "[CivilCalc/ERROR] T: e-msg (cause)");

        reset_sink();
    }

    /// 源项目契约：**日志门面自身绝不抛异常**
    #[test]
    fn panicking_sink_does_not_propagate() {
        let _guard = TEST_LOCK.lock().unwrap();
        install(Box::new(|_, _, _, _| panic!("sink 故意 panic")));

        // 若门面未吞掉 panic，这里会 panic 导致测试失败
        e("T", "业务继续", None);
        i("T", "业务继续");
        w("T", "业务继续", None);
        d("T", "业务继续");

        reset_sink();
    }

    #[test]
    fn macros_work() {
        let _guard = TEST_LOCK.lock().unwrap();
        let collected = install_collector();

        log_d!("M", "d");
        log_i!("M", "i");
        log_w!("M", "w");
        log_e!("M", "e", "why");

        let lines = collected.lock().unwrap().clone();
        assert_eq!(lines.len(), 4);
        assert!(lines[3].ends_with("(why)"));

        reset_sink();
    }
}
