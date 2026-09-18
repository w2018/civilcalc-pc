//! 应用共享状态。
//!
//! ## 分域锁设计（见 `docs/05-项目开发方案.md` §1.4.2）
//!
//! 采用分域锁而非单一全局锁，避免**求值阻塞数据库**。
//!
//! ## 死锁预防
//!
//! 禁止在持有一把锁时申请另一把。跨域操作（如"保存公式 + 更新索引"）
//! 在命令层**顺序获取、用完即放**。
//!
//! ## 关于 `db` 的类型
//!
//! [`Db`] 内部已经是 `Arc<Mutex<Connection>>`（单连接 + Mutex），
//! 外层再套一层 `Arc` 只为让 `AppState` 可共享；**不要再套 `Mutex`**
//! —— 会形成双重加锁，且 `Db` 的方法自己会加锁。

use crate::config::AppConfig;
use crate::error::{CmdResult, CommandError};
use crate::paths::DesktopPaths;
use crate::secrets::{KeyringStore, Secrets};
use civilcalc_backup::CancelFlag;
use civilcalc_core::search::SearchIndex;
use civilcalc_store::Db;
use std::sync::{Arc, Mutex};

/// 应用状态（由 Tauri `app.manage()` 注入）
pub struct AppState {
    /// 桌面路径
    pub paths: Arc<DesktopPaths>,

    /// 本地数据库（五张活表；唯一依赖 `rusqlite` 的入口，ADR-025）
    pub db: Arc<Db>,

    /// 应用偏好（22 个键）。
    ///
    /// 需要 `Mutex`：`AppConfig` 是可变结构体（`set_str` 等要 `&mut self`）。
    /// 这是**与 `db` 不同的地方** —— `Db` 自己内部就有锁，这里没有。
    pub config: Arc<Mutex<AppConfig>>,

    /// 密钥与安全配置（Windows 凭据管理器，ADR-006）。
    ///
    /// **不需要 `Mutex`**：`Secrets` 的方法全是 `&self`（底层 keyring 自身线程安全）。
    pub secrets: Arc<Secrets<KeyringStore>>,

    /// 公式检索索引（P2-4）。
    ///
    /// 需要 `Mutex`：重建时整体替换。索引**不可变**（每次重建换一个新的），
    /// 因此读锁持有的时间很短。
    pub index: Arc<Mutex<SearchIndex>>,

    /// 长任务（AI 生成 / WebDAV 传输）的**取消标志**。
    ///
    /// 不需要 `Mutex`：内部是 `AtomicBool`，本身就是线程安全的。
    /// 与 `db` / `config` 不同 —— 那两个含可变结构体。
    pub net: Arc<NetState>,
}

/// 长任务取消状态。
///
/// ## 为什么是「标志」而不是「句柄」
///
/// 取消要**立即断开连接**（不是等请求跑完），所以标志必须能被**网络读循环**看到。
/// 命令层把 `&AtomicBool` 一路传进 `LlmClient::chat_stream`，循环每读到一个 chunk
/// 就检查一次，命中即 drop 响应体（连接随之关闭）。
///
/// ## ⚠️ 每次任务开始都要清标志
///
/// 否则「上次取消」会把下一次任务在第一个 chunk 就杀掉。见 [`NetState::begin_ai`]。
#[derive(Debug, Default)]
pub struct NetState {
    ai_cancel: std::sync::atomic::AtomicBool,

    /// WebDAV 上传/下载的取消标志。
    ///
    /// 直接用 `civilcalc_backup::CancelFlag` 而不是再包一层 `AtomicBool`：
    /// 它内部就是 `Arc<AtomicBool>`，而且**是客户端要的类型** ——
    /// 命令层可以 `clone()` 一份传进网络循环，两边共享同一个标志。
    webdav_cancel: CancelFlag,
}

impl NetState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 开始一次 AI 任务：**清掉上一次的取消标志**。
    pub fn begin_ai(&self) {
        self.ai_cancel
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// 请求取消当前 AI 任务（`ai_cancel` 命令）。
    pub fn cancel_ai(&self) {
        self.ai_cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// 当前是否已请求取消。
    #[must_use]
    pub fn ai_cancelled(&self) -> bool {
        self.ai_cancel.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// 借出标志本身（传给流式客户端逐 chunk 检查）。
    #[must_use]
    pub fn ai_flag(&self) -> &std::sync::atomic::AtomicBool {
        &self.ai_cancel
    }

    /// 开始一次 WebDAV 传输：**清掉上一次的取消标志**。
    ///
    /// ⚠️ 与 [`NetState::begin_ai`] 同理 —— 不清的话，上次取消会把新任务
    /// 在第一个分块就杀掉（上传表现为「秒失败」，最难查的一类）。
    pub fn begin_webdav(&self) -> CancelFlag {
        self.webdav_cancel.reset();
        self.webdav_cancel.clone()
    }

    /// 请求取消当前 WebDAV 传输（`webdav_cancel` 命令）。
    pub fn cancel_webdav(&self) {
        self.webdav_cancel.cancel();
    }

    /// 当前是否已请求取消 WebDAV 传输。
    #[must_use]
    pub fn webdav_cancelled(&self) -> bool {
        self.webdav_cancel.is_cancelled()
    }
}

impl AppState {
    pub fn new(paths: DesktopPaths, db: Db, config: AppConfig, index: SearchIndex) -> Self {
        Self {
            paths: Arc::new(paths),
            db: Arc::new(db),
            config: Arc::new(Mutex::new(config)),
            secrets: Arc::new(Secrets::new(KeyringStore::default())),
            index: Arc::new(Mutex::new(index)),
            net: Arc::new(NetState::new()),
        }
    }

    /// 用当前库里的全部公式**重建**检索索引，返回索引条数。
    ///
    /// 公式数量是几十到几百量级，全量重建的成本远低于维护增量索引的复杂度。
    pub fn rebuild_index(&self) -> CmdResult<usize> {
        let formulas = self.db.list_formulas()?;
        let fresh = SearchIndex::build(&formulas);
        let n = fresh.len();
        let mut g = self.index.lock().map_err(|_| CommandError::Storage {
            message: "检索索引锁已中毒，请重启应用".to_string(),
        })?;
        *g = fresh;
        Ok(n)
    }

    /// 只读访问索引（拿锁 → 执行 → 放锁）
    pub fn with_index<T>(&self, f: impl FnOnce(&SearchIndex) -> T) -> CmdResult<T> {
        let g = self.index.lock().map_err(|_| CommandError::Storage {
            message: "检索索引锁已中毒，请重启应用".to_string(),
        })?;
        Ok(f(&g))
    }

    // ---------------------------------------------------------------- 配置访问

    /// 只读访问偏好。
    ///
    /// 锁中毒时返回 [`CommandError::Storage`]，**不 panic** ——
    /// 一个线程 panic 不该让整个应用不可用。
    pub fn with_config<T>(&self, f: impl FnOnce(&AppConfig) -> T) -> CmdResult<T> {
        let g = self.config.lock().map_err(|_| CommandError::Storage {
            message: "偏好数据锁已中毒，请重启应用".to_string(),
        })?;
        Ok(f(&g))
    }

    /// 修改偏好并**立即落盘**。
    ///
    /// 落盘失败会把错误抛给调用方，但**内存里的修改已生效**
    /// （下次再改会再试一次落盘）—— 这比"改不成就整个失败"对用户更友好。
    pub fn update_config<T>(&self, f: impl FnOnce(&mut AppConfig) -> T) -> CmdResult<T> {
        let (result, snapshot) = {
            let mut g = self.config.lock().map_err(|_| CommandError::Storage {
                message: "偏好数据锁已中毒，请重启应用".to_string(),
            })?;
            let r = f(&mut g);
            (r, g.clone())
        };
        snapshot.save()?;
        Ok(result)
    }
}
