//! SQLite 数据访问（**五张活表** + `user_version` 迁移）。
//!
//! 依据：`docs/04-数据契约.md` §3
//!
//! ## 硬约束
//!
//! - **全仓库唯一使用 `rusqlite` 的地方**（见 `docs/05` §3.1.2「数据访问集中」）
//! - **不得依赖 `tauri`**（ADR-025）—— 由 `Cargo.toml` 天然强制
//! - 表结构与源项目 Room **1:1 对齐**（保证备份包数据映射一致，ADR-009）
//! - 迁移用 `PRAGMA user_version` 逐级升级，**每次迁移在事务内完成**
//! - **表列名用 camelCase**（对齐源项目 Room 的默认行为，减少映射层出错）
//!
//! ## ⚠️ JSON 列保持裸字符串（备份保真，ADR-009）
//!
//! `history` 与 `formula_versions` 的 JSON 列**原样存取**，不做
//! "解析成结构体 → 再序列化" —— 那会改变字段顺序与浮点格式（`1.0` vs `1`），
//! 破坏备份包与 Android 端的逐字节兼容。
//! 需要结构体时用 [`HistoryEntry::inputs`] / [`FormulaVersion::schema`] 等**访问器**。
//!
//! ## 错误类型映射（不新增 `CoreError` 变体）
//!
//! `CoreError` 的 12 个变体**逐一对齐**源项目 `AppError`（9 个）+ PC 端 3 个新增，
//! 这是跨端错误文案一致的前提（`docs/04-数据契约.md` §7.1）。本模块**不扩变体**，
//! 按下表复用：
//!
//! | 本模块的场景 | 用哪个变体 | 理由 |
//! |---|---|---|
//! | 开库 / `execute` / `prepare` / `query` 失败 | [`CoreError::Storage`] | 对齐 `AppError.Storage` / `STORAGE_ERROR` |
//! | DB 列里的 JSON 解不开 / 序列化失败 | [`CoreError::Parse`] | 对齐 `AppError.Parse` / `PARSE_ERROR`，文案是"解析××失败" |
//! | 按主键查不到（如 `get_version`） | [`CoreError::NotFound`] | 对齐 `AppError.NotFound` / `NOT_FOUND` |
//!
//! ## ⚠️ 为什么是五张表而不是源项目的七张（ADR-024）
//!
//! 源项目 `CivilCalcDatabase` 声明了 7 个实体，但其中两张**已是死表**
//! （源项目 `data/backup/BackupMapping.kt:20` 亲口承认「随功能下架已成死表」）：
//!
//! | 死表 | 被谁取代 | 证据 |
//! |---|---|---|
//! | `report_templates` | `ExportOptions`（落 DataStore 偏好） | `templateList`/`templateSave`/`templateDelete`/`templateSetActive` **全仓库零调用点** |
//! | `llm_profiles` | 加密存储里的 `llm_config` JSON | `llmListProfiles`/`llmSaveProfile`/`llmSetActive` **全仓库零调用点** |
//!
//! 并且这两张表**不进备份包**：`BackupTables` 只有
//! `formulas` / `history` / `favorites` / `versions` / `usageStats` 五个字段。
//!
//! 源项目留着它们只是「为了不改 schema」；PC 端是**全新安装**、没有迁移包袱，
//! 因此直接不建 —— 少两张空表，也避免后来者误以为它们是活的。
//!
//! > 注意：`ReportTemplate` / `LlmProfile` 这些**类型仍然存在**（分别由
//! > `civilcalc-report` 的 `ExportOptions::to_template()` 与 `civilcalc-llm`
//! > 的配置解析使用），只是不再落库。
//!
//! ## 与源项目的其它差异
//!
//! - 源项目 DB version 3；PC 端 v1 一次性建全五张表
//! - 额外加 `user_formulas.headVersion` 列（ADR-014，源项目缺 head 语义）
//! - **从 Android 导入数据走备份包**，不直接拷 DB（迁移历史不同）

use civilcalc_core::schema::{FormulaSchema, FormulaVersion, HistoryEntry};
use civilcalc_core::CoreError;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// 当前数据库版本。新增迁移时 +1 并在 [`Db::migrate`] 中追加分支。
pub const DB_VERSION: i32 = 1;

/// 数据访问层。
///
/// 用**单连接 + `Mutex`**（桌面单用户、短事务足够；避免多连接写锁竞争）。
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// 打开数据库并运行迁移。
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Storage { message: format!("创建数据目录失败: {e}") })?;
        }
        let conn = Connection::open(path)
            .map_err(|e| CoreError::Storage { message: format!("打开数据库失败: {e}") })?;
        Self::init(conn)
    }

    /// 打开内存数据库（测试用）。
    pub fn open_in_memory() -> Result<Self, CoreError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| CoreError::Storage { message: format!("打开内存数据库失败: {e}") })?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, CoreError> {
        // WAL：读写并发；NORMAL：WAL 下的安全/性能平衡点
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;",
        )
        .map_err(|e| CoreError::Storage { message: format!("设置 PRAGMA 失败: {e}") })?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    /// 当前 schema 版本（供"关于"页与排障展示）
    pub fn user_version(&self) -> Result<i32, CoreError> {
        let conn = self.lock()?;
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| CoreError::Storage { message: e.to_string() })
    }

    /// WAL checkpoint（备份前调用，保证 `.db` 自包含）
    pub fn checkpoint(&self) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| CoreError::Storage { message: format!("checkpoint 失败: {e}") })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, CoreError> {
        self.conn
            .lock()
            .map_err(|_| CoreError::Storage { message: "数据库连接锁已中毒".to_string() })
    }

    // =========================================================================
    // 迁移
    // =========================================================================

    fn migrate(&self) -> Result<(), CoreError> {
        let mut conn = self.lock()?;
        let current: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);

        if current >= DB_VERSION {
            return Ok(());
        }

        let tx = conn
            .transaction()
            .map_err(|e| CoreError::Storage { message: format!("开启迁移事务失败: {e}") })?;

        if current < 1 {
            create_v1(&tx)?;
        }
        // 后续迁移在此追加：if current < 2 { ... }

        tx.pragma_update(None, "user_version", DB_VERSION)
            .map_err(|e| CoreError::Storage { message: format!("写入 user_version 失败: {e}") })?;
        tx.commit()
            .map_err(|e| CoreError::Storage { message: format!("提交迁移事务失败: {e}") })?;

        civilcalc_core::log::i(
            "Db",
            &format!("数据库已从 v{current} 迁移到 v{DB_VERSION}"),
        );
        Ok(())
    }
}

/// v1：一次性建全五张活表（PC 端为全新安装）。
///
/// ⚠️ 列名用 **camelCase**（对齐源项目 Room）。
///
/// 表清单 = 备份包 `BackupTables` 的五个字段，一一对应：
/// `user_formulas` / `history` / `favorites` / `formula_versions` / `llm_usage_stats`。
fn create_v1(tx: &rusqlite::Transaction<'_>) -> Result<(), CoreError> {
    tx.execute_batch(
        r#"
        -- 1. 用户公式（含内置库播种结果）
        CREATE TABLE IF NOT EXISTS user_formulas (
            id           TEXT PRIMARY KEY,
            schemaJson   TEXT NOT NULL,
            favorite     INTEGER NOT NULL DEFAULT 0,
            createdAt    INTEGER NOT NULL,
            updatedAt    INTEGER NOT NULL,
            headVersion  TEXT DEFAULT NULL
        );

        -- 2. 计算历史
        CREATE TABLE IF NOT EXISTS history (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            formulaId            TEXT NOT NULL,
            formulaSnapshotJson  TEXT NOT NULL,
            inputsJson           TEXT NOT NULL,
            resultJson           TEXT NOT NULL,
            thinkingContent      TEXT DEFAULT NULL,
            createdAt            INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS index_history_formulaId ON history (formulaId);
        CREATE INDEX IF NOT EXISTS index_history_createdAt ON history (createdAt);

        -- 3. 收藏（与 user_formulas.favorite 冗余，但两者语义不同：
        --    前者是"收藏时间序"，后者是"公式上的标记位"，源项目两边都写）
        CREATE TABLE IF NOT EXISTS favorites (
            formulaId  TEXT PRIMARY KEY,
            createdAt  INTEGER NOT NULL
        );

        -- 4. 公式版本
        CREATE TABLE IF NOT EXISTS formula_versions (
            formulaId      TEXT NOT NULL,
            version        TEXT NOT NULL,
            parentVersion  TEXT,
            schemaJson     TEXT NOT NULL,
            changeType     TEXT NOT NULL,
            changeLog      TEXT NOT NULL DEFAULT '',
            editor         TEXT NOT NULL,
            createdAt      INTEGER NOT NULL,
            verified       INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (formulaId, version)
        );
        CREATE INDEX IF NOT EXISTS index_formula_versions_formulaId ON formula_versions (formulaId);

        -- 5. Token 用量统计（只增不改；"清空统计"即整表 DELETE）
        CREATE TABLE IF NOT EXISTS llm_usage_stats (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            modelLabel        TEXT NOT NULL,
            promptTokens      INTEGER NOT NULL,
            completionTokens  INTEGER NOT NULL,
            totalTokens       INTEGER NOT NULL,
            cachedTokens      INTEGER NOT NULL,
            reasoningTokens   INTEGER NOT NULL,
            createdAt         INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS index_llm_usage_stats_modelLabel ON llm_usage_stats (modelLabel);
        CREATE INDEX IF NOT EXISTS index_llm_usage_stats_createdAt ON llm_usage_stats (createdAt);
        "#,
    )
    .map_err(|e| CoreError::Storage { message: format!("建表失败: {e}") })?;
    Ok(())
}

// =============================================================================
// user_formulas
// =============================================================================

impl Db {
    /// 保存公式（1:1 对齐源项目 `FormulaRepositoryImpl.saveSchema`）。
    ///
    /// ## 时间戳口径（对齐源项目，**不要改成 `now()`**）
    ///
    /// 源项目 `saveSchema`：
    ///
    /// ```kotlin
    /// val existing = formulaDao.getById(schema.id)
    /// val entity = UserFormulaEntity(
    ///     favorite  = existing?.favorite ?: 0,
    ///     createdAt = existing?.createdAt ?: schema.createdAt,
    ///     updatedAt = schema.updatedAt,          // ★ 用 schema 自己的，不是 now()
    /// )
    /// ```
    ///
    /// 即**调用方负责在保存前把 `schema.updatedAt` 设成当前时间**，数据库忠实照抄。
    /// 这样 `schemaJson` 里的时间戳与行上的 `updatedAt` 永远一致。
    ///
    /// ## 两处对源项目缺陷的修正
    ///
    /// - 源项目用 Room `REPLACE` 整行覆盖，靠"先查旧记录"保住 `favorite`（BUG-23）；
    ///   这里用 `ON CONFLICT DO UPDATE` **只更新 schemaJson 与 updatedAt**，
    ///   效果相同但少一次查询，且**结构上不可能丢收藏**
    ///   （回归测试 `upsert_preserves_favorite_flag`）
    /// - `headVersion` 只在新建行时写 `NULL`；**更新时不动**（否则切版本记录会被清掉）
    pub fn upsert_formula(&self, schema: &FormulaSchema) -> Result<(), CoreError> {
        let json = serde_json::to_string(schema)
            .map_err(|e| CoreError::Parse { message: format!("序列化公式失败: {e}") })?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO user_formulas (id, schemaJson, favorite, createdAt, updatedAt, headVersion)
             VALUES (?1, ?2, 0, ?3, ?4, NULL)
             ON CONFLICT(id) DO UPDATE SET schemaJson = ?2, updatedAt = ?4",
            params![schema.id, json, schema.created_at, schema.updated_at],
        )
        .map_err(|e| CoreError::Storage { message: format!("保存公式失败: {e}") })?;
        Ok(())
    }

    pub fn get_formula(&self, id: &str) -> Result<Option<FormulaSchema>, CoreError> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row(
                "SELECT schemaJson FROM user_formulas WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        decode_opt(json, "公式")
    }

    pub fn list_formulas(&self) -> Result<Vec<FormulaSchema>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT schemaJson FROM user_formulas ORDER BY updatedAt DESC")
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let mut out = Vec::new();
        for r in rows {
            let json = r.map_err(|e| CoreError::Storage { message: e.to_string() })?;
            out.push(
                serde_json::from_str(&json)
                    .map_err(|e| CoreError::Parse { message: format!("解析公式失败: {e}") })?,
            );
        }
        Ok(out)
    }

    pub fn delete_formula(&self, id: &str) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM user_formulas WHERE id = ?1", params![id])
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        Ok(())
    }

    pub fn formula_exists(&self, id: &str) -> Result<bool, CoreError> {
        let conn = self.lock()?;
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM user_formulas WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        Ok(n > 0)
    }

    /// 内置库播种：仅在表为空时插入（幂等）
    pub fn seed_builtins_if_empty(&self, formulas: &[FormulaSchema]) -> Result<usize, CoreError> {
        let conn = self.lock()?;
        let n: i64 = conn
            .query_row("SELECT COUNT(1) FROM user_formulas", [], |r| r.get(0))
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        if n > 0 {
            return Ok(0);
        }
        drop(conn);

        let mut inserted = 0usize;
        for f in formulas {
            self.upsert_formula(f)?;
            inserted += 1;
        }
        Ok(inserted)
    }

    // ---- head 版本（ADR-014）----

    pub fn get_head_version(&self, formula_id: &str) -> Result<Option<String>, CoreError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT headVersion FROM user_formulas WHERE id = ?1",
            params![formula_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|o| o.flatten())
        .map_err(|e| CoreError::Storage { message: e.to_string() })
    }

    pub fn set_head_version(&self, formula_id: &str, version: &str) -> Result<(), CoreError> {
        let conn = self.lock()?;
        let n = conn
            .execute(
                "UPDATE user_formulas SET headVersion = ?1, updatedAt = ?2 WHERE id = ?3",
                params![version, now_ms(), formula_id],
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        if n == 0 {
            return Err(CoreError::NotFound { message: format!("公式不存在: {formula_id}") });
        }
        Ok(())
    }
}

// =============================================================================
// favorites
// =============================================================================

impl Db {
    pub fn set_favorite(&self, formula_id: &str, favorite: bool) -> Result<(), CoreError> {
        let conn = self.lock()?;
        let now = now_ms();
        if favorite {
            conn.execute(
                "INSERT OR REPLACE INTO favorites (formulaId, createdAt) VALUES (?1, ?2)",
                params![formula_id, now],
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
            conn.execute(
                "UPDATE user_formulas SET favorite = 1, updatedAt = ?1 WHERE id = ?2",
                params![now, formula_id],
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        } else {
            conn.execute(
                "DELETE FROM favorites WHERE formulaId = ?1",
                params![formula_id],
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
            conn.execute(
                "UPDATE user_formulas SET favorite = 0, updatedAt = ?1 WHERE id = ?2",
                params![now, formula_id],
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        }
        Ok(())
    }

    pub fn is_favorite(&self, formula_id: &str) -> Result<bool, CoreError> {
        let conn = self.lock()?;
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM favorites WHERE formulaId = ?1",
                params![formula_id],
                |r| r.get(0),
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        Ok(n > 0)
    }

    pub fn list_favorite_ids(&self) -> Result<Vec<String>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT formulaId FROM favorites ORDER BY createdAt DESC")
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        collect_rows(rows)
    }

    pub fn list_favorite_formulas(&self) -> Result<Vec<FormulaSchema>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT f.schemaJson FROM user_formulas f
                 JOIN favorites v ON v.formulaId = f.id
                 ORDER BY v.createdAt DESC",
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let mut out = Vec::new();
        for r in rows {
            let json = r.map_err(|e| CoreError::Storage { message: e.to_string() })?;
            out.push(
                serde_json::from_str(&json)
                    .map_err(|e| CoreError::Parse { message: format!("解析公式失败: {e}") })?,
            );
        }
        Ok(out)
    }
}

// =============================================================================
// history
// =============================================================================

impl Db {
    /// 追加一条历史。
    ///
    /// ⚠️ 三处 JSON **原样入库、不做解析再序列化** —— 备份包要求逐字节保真
    /// （ADR-009），且 `inputsJson` 可能是 `"{}"`、`resultJson` 可能是空串。
    pub fn insert_history(&self, entry: &HistoryEntry) -> Result<i64, CoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO history (formulaId, formulaSnapshotJson, inputsJson, resultJson, thinkingContent, createdAt)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                entry.formula_id,
                entry.formula_snapshot_json,
                entry.inputs_json,
                entry.result_json,
                entry.thinking_content,
                entry.created_at
            ],
        )
        .map_err(|e| CoreError::Storage { message: format!("写入历史失败: {e}") })?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_history(
        &self,
        formula_id: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<HistoryEntry>, CoreError> {
        let conn = self.lock()?;
        // ⚠️ 源项目 HistoryDao 只有 `ORDER BY createdAt DESC`。这里追加 `, id DESC`
        //    作为 **tiebreaker**：`createdAt` 是毫秒，同一次操作里"AI 解析 + 立即计算"
        //    可能落在同一毫秒，此时源项目的顺序是未定义的（SQLite 取决于查询计划）。
        //    `id` 是 AUTOINCREMENT，天然单调，用它兜底让"最新"有确定含义。
        //    这属于**确定性加固**，不改变任何有确定顺序的场景。
        let sql = if formula_id.is_some() {
            "SELECT id, formulaId, formulaSnapshotJson, inputsJson, resultJson, thinkingContent, createdAt
             FROM history WHERE formulaId = ?1 ORDER BY createdAt DESC, id DESC LIMIT ?2 OFFSET ?3"
        } else {
            "SELECT id, formulaId, formulaSnapshotJson, inputsJson, resultJson, thinkingContent, createdAt
             FROM history ORDER BY createdAt DESC, id DESC LIMIT ?1 OFFSET ?2"
        };

        let mut out = Vec::new();
        if let Some(fid) = formula_id {
            let mut stmt = conn.prepare(sql).map_err(|e| CoreError::Storage { message: e.to_string() })?;
            let rows = stmt
                .query_map(params![fid, limit, offset], map_history_row)
                .map_err(|e| CoreError::Storage { message: e.to_string() })?;
            for r in rows {
                out.push(r.map_err(|e| CoreError::Storage { message: e.to_string() })?);
            }
        } else {
            let mut stmt = conn.prepare(sql).map_err(|e| CoreError::Storage { message: e.to_string() })?;
            let rows = stmt
                .query_map(params![limit, offset], map_history_row)
                .map_err(|e| CoreError::Storage { message: e.to_string() })?;
            for r in rows {
                out.push(r.map_err(|e| CoreError::Storage { message: e.to_string() })?);
            }
        }
        Ok(out)
    }

    /// 按 id 取单条历史（**历史回填用**）。
    ///
    /// 前端「历史 → 回填复现」的链路是：
    /// 路由只带 `formulaId` + `historyId`（**不把大 JSON 塞进路由**），
    /// 工作台拿到 `historyId` 后用本方法取回 `inputsJson` 预填参数。
    ///
    /// 不存在时返回 `Ok(None)` —— 历史可能已被清理，这不是错误。
    pub fn get_history(&self, id: i64) -> Result<Option<HistoryEntry>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, formulaId, formulaSnapshotJson, inputsJson, resultJson, thinkingContent, createdAt
                 FROM history WHERE id = ?1",
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;

        let mut rows = stmt
            .query_map(params![id], map_history_row)
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;

        match rows.next() {
            Some(r) => Ok(Some(
                r.map_err(|e| CoreError::Storage { message: e.to_string() })?,
            )),
            None => Ok(None),
        }
    }

    pub fn delete_history(&self, id: i64) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM history WHERE id = ?1", params![id])
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        Ok(())
    }

    pub fn clear_history(&self) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM history", [])
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        Ok(())
    }

    /// 历史记录条数（`reset_counts` 用）。
    pub fn history_count(&self) -> Result<i64, CoreError> {
        let conn = self.lock()?;
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        Ok(n)
    }

    /// 取某公式**最新一条**历史的思考内容（供「AI 思考过程」回溯）。
    ///
    /// ## 1:1 对齐源项目，**不过滤空值**
    ///
    /// 源项目 `FormulaRepositoryImpl.getLatestThinkingContent`：
    ///
    /// ```kotlin
    /// val list = historyDao.getByFormulaId(formulaId, limit = 1, offset = 0).first()
    /// list.firstOrNull()?.thinkingContent     // ← 直接取最新一条，不做非空过滤
    /// ```
    ///
    /// 也就是说：**最新那条历史没有思考内容时，返回 `None`**
    /// （例如用户刚做过一次「不调 AI」的纯计算）。
    ///
    /// ⚠️ 初版实现曾加 `thinkingContent IS NOT NULL AND != ''` 过滤（"跳过空值取最近一次
    /// 有思考的"），那是**偏离源行为**的，已改回 1:1。若将来确实需要"最近一次有思考的"
    /// 语义，请另开方法并显式命名，不要改这个。
    pub fn latest_thinking(&self, formula_id: &str) -> Result<Option<String>, CoreError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT thinkingContent FROM history
             WHERE formulaId = ?1
             ORDER BY createdAt DESC, id DESC LIMIT 1",
            params![formula_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|o| o.flatten())
        .map_err(|e| CoreError::Storage { message: e.to_string() })
    }
}

// =============================================================================
// formula_versions
// =============================================================================

impl Db {
    /// 写入一条版本。同主键覆盖（`INSERT OR REPLACE`，对齐源项目 `versionDao.insert`）。
    ///
    /// ⚠️ `schemaJson` 原样入库（备份保真），不解析再序列化。
    pub fn insert_version(&self, v: &FormulaVersion) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO formula_versions
             (formulaId, version, parentVersion, schemaJson, changeType, changeLog, editor, createdAt, verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                v.formula_id,
                v.version,
                v.parent_version,
                v.schema_json,
                v.change_type,
                v.change_log,
                v.editor,
                v.created_at,
                if v.verified { 1 } else { 0 }
            ],
        )
        .map_err(|e| CoreError::Storage { message: format!("写入版本失败: {e}") })?;
        Ok(())
    }

    pub fn list_versions(&self, formula_id: &str) -> Result<Vec<FormulaVersion>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT formulaId, version, parentVersion, schemaJson, changeType, changeLog, editor, createdAt, verified
                 FROM formula_versions WHERE formulaId = ?1 ORDER BY createdAt DESC",
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let rows = stmt
            .query_map(params![formula_id], map_version_row)
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CoreError::Storage { message: e.to_string() })?);
        }
        Ok(out)
    }

    pub fn get_version(&self, formula_id: &str, version: &str) -> Result<FormulaVersion, CoreError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT formulaId, version, parentVersion, schemaJson, changeType, changeLog, editor, createdAt, verified
             FROM formula_versions WHERE formulaId = ?1 AND version = ?2",
            params![formula_id, version],
            map_version_row,
        )
        .optional()
        .map_err(|e| CoreError::Storage { message: e.to_string() })?
        .ok_or_else(|| CoreError::NotFound { message: format!("版本不存在: {formula_id} / {version}") })
    }
}

// =============================================================================
// （已移除）report_templates
// =============================================================================
//
// 源项目该表已是死表（见模块文档 ADR-024）：`templateList`/`templateSave`/
// `templateDelete`/`templateSetActive` 全仓库零调用点，能力已被
// `ExportOptions`（落 DataStore 偏好）取代，且不进备份包。
//
// 计算书章节模型 `ReportTemplate` / `TemplateSection` 仍然存在，
// 但只作为 `civilcalc-report` 里 `ExportOptions::to_template()` 的返回值，
// 不落库 —— 因此这里没有 upsert/list/get/delete。

// =============================================================================
// （已移除）llm_profiles
// =============================================================================
//
// 同理（ADR-024）：`llmListProfiles`/`llmSaveProfile`/`llmSetActive`
// 全仓库零调用点。真实模型配置（地址 / 模型 / 思考强度 / 协议 / 视觉 / 活跃模型）
// 存在加密存储的 `llm_config` JSON 里，走 `BackupSection::LLM_CONFIG`
// 原样搬运；密钥单独走 keyring（见 P1-8 `secrets.rs`）。
//
// ⚠️ 因此 `llm_profiles` 表**不是** API Key 的家，也从来不是 ——
// 源项目该表的 `apiKeyPlaceholder` 恒为空串（BUG-27 凭据审计）。

// =============================================================================
// llm_usage_stats
// =============================================================================

/// 一条用量记录
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub model_label: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
}

/// 按模型汇总的用量
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub model_label: String,
    pub total_prompt: i64,
    pub total_completion: i64,
    pub total_all: i64,
    pub total_cached: i64,
    pub total_reasoning: i64,
    pub call_count: i64,
}

/// 按**本地日期**汇总的用量（`day` 形如 `"2026-09-17"`）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDaySummary {
    /// 本地日期（`YYYY-MM-DD`）
    pub day: String,
    pub total_prompt: i64,
    pub total_completion: i64,
    pub total_all: i64,
    pub total_cached: i64,
    pub total_reasoning: i64,
    pub call_count: i64,
}

/// 单条用量记录（CSV 导出用）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub id: i64,
    pub model_label: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub created_at: i64,
}

impl Db {
    /// 追加一条用量记录
    pub fn insert_usage(&self, row: &UsageRow) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO llm_usage_stats
             (modelLabel, promptTokens, completionTokens, totalTokens, cachedTokens, reasoningTokens, createdAt)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.model_label,
                row.prompt_tokens,
                row.completion_tokens,
                row.total_tokens,
                row.cached_tokens,
                row.reasoning_tokens,
                now_ms()
            ],
        )
        .map_err(|e| CoreError::Storage { message: format!("写入用量失败: {e}") })?;
        Ok(())
    }

    /// 按模型汇总
    pub fn usage_summary(&self) -> Result<Vec<UsageSummary>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT modelLabel,
                        SUM(promptTokens), SUM(completionTokens), SUM(totalTokens),
                        SUM(cachedTokens), SUM(reasoningTokens), COUNT(1)
                 FROM llm_usage_stats GROUP BY modelLabel ORDER BY SUM(totalTokens) DESC",
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let rows = stmt
            .query_map([], |r| {
                Ok(UsageSummary {
                    model_label: r.get(0)?,
                    total_prompt: r.get(1)?,
                    total_completion: r.get(2)?,
                    total_all: r.get(3)?,
                    total_cached: r.get(4)?,
                    total_reasoning: r.get(5)?,
                    call_count: r.get(6)?,
                })
            })
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CoreError::Storage { message: e.to_string() })?);
        }
        Ok(out)
    }

    /// 按**本地日期**汇总（`"2026-09-17"`）。
    ///
    /// SQLite 侧用 `date(createdAt/1000, 'unixepoch', 'localtime')`：
    /// `createdAt` 存的是**毫秒**，先转秒再让 SQLite 按本地时区取日期 ——
    /// 不要用 Rust 侧 `chrono` 再分组，那会多一次全表遍历。
    ///
    /// 按日期**倒序**（最近的在前）。
    pub fn usage_summary_by_day(&self) -> Result<Vec<UsageDaySummary>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT date(createdAt / 1000, 'unixepoch', 'localtime') AS day,
                        SUM(promptTokens), SUM(completionTokens), SUM(totalTokens),
                        SUM(cachedTokens), SUM(reasoningTokens), COUNT(1)
                 FROM llm_usage_stats GROUP BY day ORDER BY day DESC",
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let rows = stmt
            .query_map([], |r| {
                Ok(UsageDaySummary {
                    day: r.get(0)?,
                    total_prompt: r.get(1)?,
                    total_completion: r.get(2)?,
                    total_all: r.get(3)?,
                    total_cached: r.get(4)?,
                    total_reasoning: r.get(5)?,
                    call_count: r.get(6)?,
                })
            })
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CoreError::Storage { message: e.to_string() })?);
        }
        Ok(out)
    }

    /// 全部用量明细（按时间**正序**，CSV 导出用）
    pub fn list_usage(&self) -> Result<Vec<UsageRecord>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, modelLabel, promptTokens, completionTokens, totalTokens,
                        cachedTokens, reasoningTokens, createdAt
                 FROM llm_usage_stats ORDER BY createdAt ASC, id ASC",
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let rows = stmt
            .query_map([], |r| {
                Ok(UsageRecord {
                    id: r.get(0)?,
                    model_label: r.get(1)?,
                    prompt_tokens: r.get(2)?,
                    completion_tokens: r.get(3)?,
                    total_tokens: r.get(4)?,
                    cached_tokens: r.get(5)?,
                    reasoning_tokens: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CoreError::Storage { message: e.to_string() })?);
        }
        Ok(out)
    }

    /// 用量记录条数（`reset_counts` 用）
    pub fn usage_count(&self) -> Result<i64, CoreError> {
        let conn = self.lock()?;
        conn.query_row("SELECT COUNT(1) FROM llm_usage_stats", [], |r| r.get(0))
            .map_err(|e| CoreError::Storage { message: e.to_string() })
    }

    pub fn clear_usage(&self) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM llm_usage_stats", [])
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        Ok(())
    }
}

// =============================================================================
// 备份读写（P4-14）
// =============================================================================
//
// 备份格式存的是**表行的原始形态**（含 `createdAt` 这类域对象不带的时间戳），
// 而常规 API 走域对象。这里补齐「保真读写」需要的几个口子。
//
// ## 🔴 三条「不能用常规 API 顶替」的理由
//
// | 场景 | 常规 API 的问题 |
// |---|---|
// | 收藏时间 | `set_favorite` 把 `createdAt` 写成**当前时间**（还顺带改 `updatedAt`） |
// | 历史 id | `insert_history` 让 SQLite 重新分配 id，而图片的 `refs` 里有 `hist:…:<entryId>` |
// | 用量时间 | `insert_usage` 把 `createdAt` 写成当前时刻 → 历史用量全堆到「今天」 |
//
// 这类「值被悄悄换掉」的 bug 不报错、不崩溃，只是数据慢慢失真 —— 所以单独立测。

/// 收藏表的一行（`favorites.formulaId` + `favorites.createdAt`）。
///
/// 单独一个类型而不是 `(String, i64)`：两个字段类型不同还好，一旦将来加字段，
/// 元组很容易接反 —— 而接反的后果是「收藏时间变成一段哈希」这种静默错数据。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteTableRow {
    pub formula_id: String,
    pub created_at: i64,
}

impl Db {
    /// 全部收藏行（含 `createdAt`）—— 备份导出用。
    ///
    /// ⚠️ `list_favorite_ids` 只有 id，**丢了 `createdAt`**；备份格式要保真，
    /// 所以单开一个方法而不是在调用处补齐（补齐也补不出时间）。
    pub fn list_favorite_rows(&self) -> Result<Vec<FavoriteTableRow>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT formulaId, createdAt FROM favorites ORDER BY createdAt DESC, formulaId")
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let rows = stmt
            .query_map([], |r| {
                Ok(FavoriteTableRow {
                    formula_id: r.get(0)?,
                    created_at: r.get(1)?,
                })
            })
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CoreError::Storage { message: e.to_string() })?);
        }
        Ok(out)
    }

    /// 恢复一条收藏行，**保留原始 `createdAt`** —— 备份导入用。
    ///
    /// 同时把 `user_formulas.favorite` 置 1（两处必须一致，否则「收藏」页与
    /// 公式卡片上的星标会打架）。**不动 `updatedAt`** —— 恢复数据不是「改公式」。
    pub fn upsert_favorite_row(&self, row: &FavoriteTableRow) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO favorites (formulaId, createdAt) VALUES (?1, ?2)",
            params![row.formula_id, row.created_at],
        )
        .map_err(|e| CoreError::Storage { message: format!("恢复收藏失败: {e}") })?;
        conn.execute(
            "UPDATE user_formulas SET favorite = 1 WHERE id = ?1",
            params![row.formula_id],
        )
        .map_err(|e| CoreError::Storage { message: format!("更新收藏标记失败: {e}") })?;
        Ok(())
    }

    /// 全部公式版本（跨公式）—— 备份导出用。
    ///
    /// 排序 `formulaId, createdAt, version`：让产物**稳定可 diff**。
    /// 源项目 `versionDao.getAll()` 没有 ORDER BY（顺序由查询计划决定），
    /// 那会让同一次导出两次得到不同的包字节。
    pub fn list_all_versions(&self) -> Result<Vec<FormulaVersion>, CoreError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT formulaId, version, parentVersion, schemaJson, changeType, changeLog, editor, createdAt, verified
                 FROM formula_versions ORDER BY formulaId, createdAt, version",
            )
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let rows = stmt
            .query_map([], map_version_row)
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CoreError::Storage { message: e.to_string() })?);
        }
        Ok(out)
    }

    /// 写入历史并**保留原始 id** —— 备份导入用。
    ///
    /// 🔴 为什么必须保留 id：图片的 `refs` 里存的是 `hist:search:<entryId>`，
    /// id 被重新分配后，那些引用就全指向不存在（或错误的）历史条目 ——
    /// 恢复出来的图片会「挂在别人的历史下」。
    ///
    /// `id <= 0` 时（[`HistoryEntry`] 的「未入库」约定）退回普通插入，由 SQLite 分配。
    pub fn insert_history_with_id(&self, entry: &HistoryEntry) -> Result<i64, CoreError> {
        if entry.id <= 0 {
            return self.insert_history(entry);
        }
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO history
             (id, formulaId, formulaSnapshotJson, inputsJson, resultJson, thinkingContent, createdAt)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id,
                entry.formula_id,
                entry.formula_snapshot_json,
                entry.inputs_json,
                entry.result_json,
                entry.thinking_content,
                entry.created_at
            ],
        )
        .map_err(|e| CoreError::Storage { message: format!("恢复历史失败: {e}") })?;
        Ok(entry.id)
    }

    /// 写入用量并**保留原始 `createdAt`** —— 备份导入用。
    ///
    /// ⚠️ 不能用 `insert_usage`：它把时间写成当前时刻，
    /// 导入后「按天统计」会把整段历史用量堆到今天那一根柱子上。
    pub fn insert_usage_at(&self, row: &UsageRow, created_at: i64) -> Result<(), CoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO llm_usage_stats
             (modelLabel, promptTokens, completionTokens, totalTokens, cachedTokens, reasoningTokens, createdAt)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.model_label,
                row.prompt_tokens,
                row.completion_tokens,
                row.total_tokens,
                row.cached_tokens,
                row.reasoning_tokens,
                created_at
            ],
        )
        .map_err(|e| CoreError::Storage { message: format!("恢复用量失败: {e}") })?;
        Ok(())
    }

    /// 清空公式（**含收藏与版本**）—— 完整还原用。
    ///
    /// ⚠️ 三张表必须一起清：只清 `user_formulas` 会留下
    /// **指向不存在公式的收藏与版本**（界面会显示「幽灵」条目，点进去报 notFound）。
    /// 用事务包起来，避免中途失败留下半清状态。
    pub fn clear_formulas(&self) -> Result<(), CoreError> {
        let mut conn = self.lock()?;
        let tx = conn
            .transaction()
            .map_err(|e| CoreError::Storage { message: e.to_string() })?;
        for sql in [
            "DELETE FROM formula_versions",
            "DELETE FROM favorites",
            "DELETE FROM user_formulas",
        ] {
            tx.execute(sql, [])
                .map_err(|e| CoreError::Storage { message: format!("{sql} 失败: {e}") })?;
        }
        tx.commit()
            .map_err(|e| CoreError::Storage { message: format!("清空公式失败: {e}") })?;
        Ok(())
    }
}

// =============================================================================
// 行映射辅助
// =============================================================================

/// `history` 行映射。三处 JSON 列**原样取出为 String**（备份保真，见 [`HistoryEntry`]）。
fn map_history_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: r.get(0)?,
        formula_id: r.get(1)?,
        formula_snapshot_json: r.get(2)?,
        inputs_json: r.get(3)?,
        result_json: r.get(4)?,
        thinking_content: r.get(5)?,
        created_at: r.get(6)?,
    })
}

/// `formula_versions` 行映射。`schemaJson` 原样取出为 String（备份保真）。
fn map_version_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FormulaVersion> {
    Ok(FormulaVersion {
        formula_id: r.get(0)?,
        version: r.get(1)?,
        parent_version: r.get(2)?,
        schema_json: r.get(3)?,
        change_type: r.get(4)?,
        change_log: r.get(5)?,
        editor: r.get(6)?,
        created_at: r.get(7)?,
        // SQLite 无 BOOLEAN，存的是 0/1（与源项目 Room `verified: Int` 一致）
        verified: r.get::<_, i64>(8)? != 0,
    })
}

fn collect_rows<T>(rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>) -> Result<Vec<T>, CoreError> {
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| CoreError::Storage { message: e.to_string() })?);
    }
    Ok(out)
}

fn decode_opt<T: serde::de::DeserializeOwned>(
    json: Option<String>,
    what: &str,
) -> Result<Option<T>, CoreError> {
    match json {
        None => Ok(None),
        Some(j) => serde_json::from_str(&j)
            .map(Some)
            .map_err(|e| CoreError::Parse { message: format!("解析{what}失败: {e}") }),
    }
}

/// 当前时间戳（Unix 毫秒）
fn now_ms() -> i64 {
    civilcalc_core::now_ms()
}

/// 便捷：把 `HashMap<String, f64>` 与 JSON 互转（供调用方使用）
pub fn inputs_to_json(inputs: &HashMap<String, f64>) -> Result<String, CoreError> {
    serde_json::to_string(inputs).map_err(|e| CoreError::Parse { message: e.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use civilcalc_core::schema::{
        AltExpression, EvalResult, FormulaSource, FormulaVar, SourceKind, StepTemplate,
        CURRENT_SCHEMA_VERSION,
    };

    fn test_db() -> Db {
        Db::open_in_memory().expect("内存库应可打开")
    }

    fn test_schema(id: &str) -> FormulaSchema {
        FormulaSchema {
            id: id.to_string(),
            result_name: "测试".to_string(),
            result_symbol: "y".to_string(),
            result_unit: Some("m".to_string()),
            result_outputs: vec![],
            expression: "a+b".to_string(),
            source_equations: vec![],
            alt_expressions: vec![],
            constants: HashMap::new(),
            variables: vec![
                FormulaVar {
                    symbol: "a".to_string(),
                    desc: "参数a".to_string(),
                    unit: Some("m".to_string()),
                    default: None,
                    min: None,
                    max: None,
                    required: true,
                },
                FormulaVar {
                    symbol: "b".to_string(),
                    desc: "参数b".to_string(),
                    unit: Some("m".to_string()),
                    default: None,
                    min: None,
                    max: None,
                    required: true,
                },
            ],
            domain: "通用".to_string(),
            tags: vec![],
            reference_basis: None,
            design_notes: None,
            explanation: None,
            image_ids: vec![],
            revised_from: None,
            source: FormulaSource::standard("GB 50010-2010 第6.2.10条"),
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

    /// 指定 `updated_at` 的测试公式（用于验证排序 / 时间戳口径）
    fn test_schema_at(id: &str, updated_at: i64) -> FormulaSchema {
        let mut s = test_schema(id);
        s.updated_at = updated_at;
        s
    }

    fn test_history(formula_id: &str, thinking: Option<&str>) -> HistoryEntry {        HistoryEntry {
            id: 0,
            formula_id: formula_id.to_string(),
            formula_snapshot_json: serde_json::to_string(&test_schema(formula_id)).unwrap(),
            inputs_json: r#"{"a":1.0,"b":2.0}"#.to_string(),
            result_json: serde_json::to_string(&EvalResult::primary_only(3.0)).unwrap(),
            thinking_content: thinking.map(|s| s.to_string()),
            created_at: now_ms(),
        }
    }

    // ---------------- 迁移 ----------------

    #[test]
    fn fresh_db_is_at_current_version() {
        let db = test_db();
        assert_eq!(db.user_version().unwrap(), DB_VERSION);
    }

    #[test]
    fn migration_is_idempotent() {
        let db = test_db();
        db.migrate().unwrap();
        db.migrate().unwrap();
        assert_eq!(db.user_version().unwrap(), DB_VERSION);
    }

    /// 五张活表必须齐全（= 备份包 `BackupTables` 的五个字段）
    #[test]
    fn all_five_live_tables_exist() {
        let db = test_db();
        let conn = db.lock().unwrap();
        for t in [
            "user_formulas",
            "history",
            "favorites",
            "formula_versions",
            "llm_usage_stats",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(1) FROM sqlite_master WHERE type='table' AND name=?1",
                    params![t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "缺少表: {t}");
        }
    }

    /// ADR-024：源项目的两张死表**不得**在 PC 端建出来。
    ///
    /// 它们零调用点、且不进备份包；建出来只会误导后来者以为功能还在。
    #[test]
    fn dead_tables_are_not_created() {
        let db = test_db();
        let conn = db.lock().unwrap();
        for t in ["report_templates", "llm_profiles"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(1) FROM sqlite_master WHERE type='table' AND name=?1",
                    params![t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{t} 是死表，不应存在（ADR-024）");
        }
    }

    #[test]
    fn user_formulas_has_head_version_column() {
        let db = test_db();
        let conn = db.lock().unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(user_formulas)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            cols.contains(&"headVersion".to_string()),
            "user_formulas 应含 headVersion 列（ADR-014），实际: {cols:?}"
        );
        // 列名用 camelCase（对齐源项目 Room）
        assert!(cols.contains(&"schemaJson".to_string()));
        assert!(cols.contains(&"createdAt".to_string()));
    }

    // ---------------- 公式 CRUD ----------------

    #[test]
    fn formula_roundtrip() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();

        let got = db.get_formula("usr:1").unwrap().unwrap();
        assert_eq!(got.id, "usr:1");
        assert_eq!(got.expression, "a+b");
        assert_eq!(got.schema_version, CURRENT_SCHEMA_VERSION);

        assert!(db.formula_exists("usr:1").unwrap());
        assert!(!db.formula_exists("usr:nope").unwrap());
        assert!(db.get_formula("usr:nope").unwrap().is_none());
    }

    /// 回归：更新公式**不得**丢失收藏状态
    /// （源项目 `save_formula` 会强制 `favorite = 0`，是已知缺陷）
    #[test]
    fn upsert_preserves_favorite_flag() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        db.set_favorite("usr:1", true).unwrap();

        let mut updated = test_schema("usr:1");
        updated.expression = "a*b".to_string();
        db.upsert_formula(&updated).unwrap();

        assert!(db.is_favorite("usr:1").unwrap(), "更新后收藏状态应保留");
        assert_eq!(db.get_formula("usr:1").unwrap().unwrap().expression, "a*b");
    }

    #[test]
    fn list_formulas_ordered_by_updated_at() {
        let db = test_db();
        db.upsert_formula(&test_schema_at("usr:1", 1000)).unwrap();
        db.upsert_formula(&test_schema_at("usr:2", 3000)).unwrap();
        db.upsert_formula(&test_schema_at("usr:3", 2000)).unwrap();

        let list = db.list_formulas().unwrap();
        let ids: Vec<&str> = list.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["usr:2", "usr:3", "usr:1"], "应按 updatedAt DESC");
    }

    /// **时间戳口径**：DB 行的 `createdAt` / `updatedAt` 取自 **schema 自己**，
    /// 不是 `now()` —— 对齐源项目 `saveSchema`
    #[test]
    fn upsert_uses_schema_timestamps_not_now() {
        let db = test_db();
        let mut s = test_schema("usr:1");
        s.created_at = 111;
        s.updated_at = 222;
        db.upsert_formula(&s).unwrap();

        let conn = db.lock().unwrap();
        let (c, u): (i64, i64) = conn
            .query_row(
                "SELECT createdAt, updatedAt FROM user_formulas WHERE id='usr:1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(c, 111, "createdAt 应取 schema.createdAt");
        assert_eq!(u, 222, "updatedAt 应取 schema.updatedAt（**不是 now()**）");
    }

    /// 冲突更新时：**保留**原 `createdAt` 与 `favorite`，**更新** `updatedAt`
    /// （对齐源项目 `saveSchema` 的 `existing?.createdAt ?: schema.createdAt`）
    #[test]
    fn upsert_on_conflict_keeps_created_at_and_favorite() {
        let db = test_db();
        let mut v1 = test_schema("usr:1");
        v1.created_at = 111;
        v1.updated_at = 222;
        db.upsert_formula(&v1).unwrap();
        db.set_favorite("usr:1", true).unwrap();

        let mut v2 = test_schema("usr:1");
        v2.created_at = 999; // 调用方改了这个，冲突更新时**不应生效**
        v2.updated_at = 333;
        v2.expression = "a*b".to_string();
        db.upsert_formula(&v2).unwrap();

        let conn = db.lock().unwrap();
        let (c, u, f): (i64, i64, i64) = conn
            .query_row(
                "SELECT createdAt, updatedAt, favorite FROM user_formulas WHERE id='usr:1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(c, 111, "createdAt 必须保留原值");
        assert_eq!(u, 333, "updatedAt 应更新为新 schema 的值");
        assert_eq!(f, 1, "favorite 必须保留");
    }

    /// 冲突更新**不得**清掉 `headVersion`（ADR-014）
    #[test]
    fn upsert_on_conflict_keeps_head_version() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        db.set_head_version("usr:1", "1.0.1").unwrap();

        db.upsert_formula(&test_schema("usr:1")).unwrap();

        assert_eq!(
            db.get_head_version("usr:1").unwrap().as_deref(),
            Some("1.0.1"),
            "重存公式不得清掉 headVersion"
        );
    }

    #[test]
    fn delete_formula_works() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        db.delete_formula("usr:1").unwrap();
        assert!(!db.formula_exists("usr:1").unwrap());
    }

    #[test]
    fn seed_builtins_is_idempotent() {
        let db = test_db();
        let builtins = vec![test_schema("builtin:1"), test_schema("builtin:2")];
        assert_eq!(db.seed_builtins_if_empty(&builtins).unwrap(), 2);
        // 第二次播种应跳过（表非空）
        assert_eq!(db.seed_builtins_if_empty(&builtins).unwrap(), 0);
        assert_eq!(db.list_formulas().unwrap().len(), 2);
    }

    // ---------------- head 版本 ----------------

    #[test]
    fn head_version_defaults_to_null() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        assert_eq!(db.get_head_version("usr:1").unwrap(), None);
    }

    #[test]
    fn head_version_switch() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        db.set_head_version("usr:1", "1.0.1").unwrap();
        assert_eq!(
            db.get_head_version("usr:1").unwrap().as_deref(),
            Some("1.0.1")
        );
    }

    #[test]
    fn head_version_on_missing_formula_is_not_found() {
        let db = test_db();
        let err = db.set_head_version("usr:nope", "1.0.0").unwrap_err();
        assert_eq!(err.code(), "NOT_FOUND");
    }

    // ---------------- 收藏 ----------------

    #[test]
    fn favorite_toggle() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();

        db.set_favorite("usr:1", true).unwrap();
        assert!(db.is_favorite("usr:1").unwrap());
        assert_eq!(db.list_favorite_ids().unwrap(), vec!["usr:1"]);
        assert_eq!(db.list_favorite_formulas().unwrap().len(), 1);

        db.set_favorite("usr:1", false).unwrap();
        assert!(!db.is_favorite("usr:1").unwrap());
        assert!(db.list_favorite_ids().unwrap().is_empty());
    }

    // ---------------- 历史 ----------------

    #[test]
    fn history_insert_and_list() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();

        let id = db.insert_history(&test_history("usr:1", Some("思考中"))).unwrap();
        assert!(id > 0);

        let list = db.list_history(Some("usr:1"), 10, 0).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].inputs()["a"], 1.0);
        assert_eq!(list[0].result().unwrap().primary, 3.0);
        assert_eq!(list[0].thinking_content.as_deref(), Some("思考中"));
        assert_eq!(list[0].id, id);
    }

    /// 备份保真：JSON 列必须**逐字节**往返，不得因解析-重序列化而变形。
    #[test]
    fn history_json_columns_roundtrip_verbatim() {
        let db = test_db();
        let mut e = test_history("usr:1", None);
        // 故意用"字段顺序非字典序 + 整数写法 1（而非 1.0）"的 JSON
        e.inputs_json = r#"{"b":2,"a":1}"#.to_string();
        e.result_json = r#"{"primary":3,"outputs":[]}"#.to_string();
        let raw_snapshot = e.formula_snapshot_json.clone();

        db.insert_history(&e).unwrap();
        let got = &db.list_history(None, 1, 0).unwrap()[0];

        assert_eq!(got.inputs_json, r#"{"b":2,"a":1}"#);
        assert_eq!(got.result_json, r#"{"primary":3,"outputs":[]}"#);
        assert_eq!(got.formula_snapshot_json, raw_snapshot);
    }

    /// AI 解析后立即记的那条历史：`inputsJson = "{}"`、`resultJson = ""`（空串）
    #[test]
    fn history_parse_only_entry_survives() {
        let db = test_db();
        let schema = test_schema("usr:1");
        let e = HistoryEntry::parse_only(&schema, Some("推理".to_string()));

        db.insert_history(&e).unwrap();
        let got = &db.list_history(Some("usr:1"), 1, 0).unwrap()[0];

        assert_eq!(got.result_json, "", "空串必须原样存回");
        assert!(got.is_parse_only());
        assert!(got.inputs().is_empty());
        assert_eq!(got.thinking_content.as_deref(), Some("推理"));
    }

    #[test]
    fn history_global_list_without_formula_filter() {
        let db = test_db();
        db.insert_history(&test_history("usr:1", None)).unwrap();
        db.insert_history(&test_history("usr:2", None)).unwrap();
        assert_eq!(db.list_history(None, 10, 0).unwrap().len(), 2);
        assert_eq!(db.list_history(Some("usr:1"), 10, 0).unwrap().len(), 1);
    }

    #[test]
    fn history_delete_one_and_clear_all() {
        let db = test_db();
        let id1 = db.insert_history(&test_history("usr:1", None)).unwrap();
        db.insert_history(&test_history("usr:1", None)).unwrap();

        db.delete_history(id1).unwrap();
        assert_eq!(db.list_history(Some("usr:1"), 10, 0).unwrap().len(), 1);

        db.clear_history().unwrap();
        assert!(db.list_history(None, 10, 0).unwrap().is_empty());
    }

    /// 1:1 对齐源项目 `getLatestThinkingContent`：
    /// 取**最新一条**历史的 `thinkingContent`，**不做非空过滤**。
    #[test]
    fn latest_thinking_matches_source_semantics() {
        let db = test_db();

        // 三条历史：有思考 → 无思考 → 有思考
        db.insert_history(&test_history("usr:1", Some("第一次思考"))).unwrap();
        db.insert_history(&test_history("usr:1", None)).unwrap();
        db.insert_history(&test_history("usr:1", Some("第二次思考"))).unwrap();

        // 最新一条（id 最大）是"第二次思考"
        assert_eq!(
            db.latest_thinking("usr:1").unwrap().as_deref(),
            Some("第二次思考")
        );

        // 再追加一条无思考的 → 最新那条没有思考内容 ⇒ None（**源项目行为，不过滤空值**）
        db.insert_history(&test_history("usr:1", None)).unwrap();
        assert_eq!(
            db.latest_thinking("usr:1").unwrap(),
            None,
            "最新一条无思考内容时应返回 None（对齐源项目，不做非空过滤）"
        );

        // 不存在的公式 → None（对齐源项目 `list.firstOrNull()` 为 null）
        assert_eq!(db.latest_thinking("usr:nope").unwrap(), None);
    }

    /// 同一毫秒内插入多条时，`id DESC` 兜底保证"最新"有确定含义
    #[test]
    fn history_same_millisecond_order_is_deterministic() {
        let db = test_db();
        // 强制三条时间戳完全相同
        for text in ["A", "B", "C"] {
            let mut e = test_history("usr:1", Some(text));
            e.created_at = 5000;
            db.insert_history(&e).unwrap();
        }

        let list = db.list_history(Some("usr:1"), 10, 0).unwrap();
        let order: Vec<_> = list
            .iter()
            .map(|h| h.thinking_content.clone().unwrap())
            .collect();
        assert_eq!(order, vec!["C", "B", "A"], "时间戳相同时按 id 降序（后插入的在前）");
        assert_eq!(db.latest_thinking("usr:1").unwrap().as_deref(), Some("C"));
    }

    // ---------------- 版本 ----------------

    /// 构造测试版本。`created_at` 固定 2000 —— 需要控制排序的测试请用
    /// [`test_version_at`]（源项目 `ORDER BY createdAt DESC` **无 tiebreaker**，
    /// 时间戳相同则顺序不确定）。
    fn test_version(formula_id: &str, version: &str, parent: Option<&str>) -> FormulaVersion {
        test_version_at(formula_id, version, parent, 2000)
    }

    fn test_version_at(
        formula_id: &str,
        version: &str,
        parent: Option<&str>,
        created_at: i64,
    ) -> FormulaVersion {
        FormulaVersion {
            formula_id: formula_id.to_string(),
            version: version.to_string(),
            parent_version: parent.map(|s| s.to_string()),
            schema_json: serde_json::to_string(&test_schema(formula_id)).unwrap(),
            change_type: "update".to_string(),
            change_log: "改动说明".to_string(),
            editor: "user".to_string(),
            created_at,
            verified: false,
        }
    }

    #[test]
    fn version_insert_and_list() {
        let db = test_db();
        // ⚠️ 源项目 `VersionDao.getByFormulaId` 是 `ORDER BY createdAt DESC`，
        //    没有 tiebreaker；时间戳相同时 SQLite 返回顺序不确定。
        //    这里给递增时间戳，验证"最新的在前"。
        db.insert_version(&test_version_at("usr:1", "1.0.0", None, 1000)).unwrap();
        db.insert_version(&test_version_at("usr:1", "1.0.1", Some("1.0.0"), 2000)).unwrap();

        let list = db.list_versions("usr:1").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].version, "1.0.1", "createdAt 最大的应排最前");
        assert_eq!(list[1].version, "1.0.0");
    }

    #[test]
    fn version_get_and_not_found() {
        let db = test_db();
        db.insert_version(&test_version("usr:1", "1.0.0", None)).unwrap();
        assert_eq!(db.get_version("usr:1", "1.0.0").unwrap().change_log, "改动说明");

        let err = db.get_version("usr:1", "9.9.9").unwrap_err();
        assert_eq!(err.code(), "NOT_FOUND");
    }

    /// `verified` 在库里必须是 0/1（与源项目 Room `verified: Int` 一致）
    #[test]
    fn version_verified_flag_maps_to_int() {
        let db = test_db();
        let mut v = test_version("usr:1", "1.0.0", None);
        v.verified = true;
        db.insert_version(&v).unwrap();

        let conn = db.lock().unwrap();
        let raw: i64 = conn
            .query_row(
                "SELECT verified FROM formula_versions WHERE formulaId='usr:1' AND version='1.0.0'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, 1);
        drop(conn);

        assert!(db.get_version("usr:1", "1.0.0").unwrap().verified);
    }

    /// 备份保真：`schemaJson` 逐字节往返（字段顺序不得被"解析再序列化"改写）
    #[test]
    fn version_schema_json_roundtrips_verbatim() {
        let db = test_db();
        // 一份**合法但字段顺序非字典序**的 FormulaSchema JSON。
        // 若实现里做了"解析成结构体再序列化"，字段顺序会变成结构体声明序，
        // 这个断言就会失败。
        let raw = r#"{"schemaVersion":3,"source":{"kind":"CUSTOM"},"domain":"通用","constants":{},"variables":[],"expression":"a+b","resultSymbol":"y","resultName":"r","id":"usr:1"}"#;

        let mut v = test_version("usr:1", "1.0.0", None);
        v.schema_json = raw.to_string();
        db.insert_version(&v).unwrap();

        let got = db.get_version("usr:1", "1.0.0").unwrap();
        assert_eq!(got.schema_json, raw, "schemaJson 必须逐字节保持");
        // 且仍可解析为结构体（访问器可用）
        let parsed = got.schema().expect("合法 FormulaSchema JSON 应可解析");
        assert_eq!(parsed.id, "usr:1");
        assert_eq!(parsed.expression, "a+b");
    }

    /// 同主键覆盖（对齐源项目 `versionDao.insert` 的 REPLACE 语义）
    #[test]
    fn version_same_key_overwrites() {
        let db = test_db();
        db.insert_version(&test_version("usr:1", "1.0.0", None)).unwrap();

        let mut v2 = test_version("usr:1", "1.0.0", None);
        v2.change_log = "改过了".to_string();
        db.insert_version(&v2).unwrap();

        let list = db.list_versions("usr:1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].change_log, "改过了");
    }

    // ---------------- （已移除）模板 / 模型配置 ----------------
    //
    // 源项目这两张表零调用点且不进备份包，PC 端不建（ADR-024），
    // 因此这里也没有对应测试。相关类型的语义测试放在各自 crate：
    // - `ReportTemplate` 章节展开 → `civilcalc-report`（P3）
    // - `LlmProfile` 配置解析    → `civilcalc-llm`（P4）

    // ---------------- 用量统计 ----------------

    fn usage(model: &str, p: i64, c: i64) -> UsageRow {
        UsageRow {
            model_label: model.to_string(),
            prompt_tokens: p,
            completion_tokens: c,
            total_tokens: p + c,
            cached_tokens: 0,
            reasoning_tokens: 0,
        }
    }

    #[test]
    fn usage_insert_and_summary_by_model() {
        let db = test_db();
        db.insert_usage(&usage("DeepSeek", 100, 50)).unwrap();
        db.insert_usage(&usage("DeepSeek", 200, 60)).unwrap();
        db.insert_usage(&usage("GLM", 10, 5)).unwrap();

        let s = db.usage_summary().unwrap();
        assert_eq!(s.len(), 2);

        let ds = s.iter().find(|x| x.model_label == "DeepSeek").unwrap();
        assert_eq!(ds.total_prompt, 300);
        assert_eq!(ds.total_completion, 110);
        assert_eq!(ds.total_all, 410);
        assert_eq!(ds.call_count, 2);

        // 按 totalTokens 降序 → DeepSeek 在前
        assert_eq!(s[0].model_label, "DeepSeek");
    }

    #[test]
    fn usage_clear() {
        let db = test_db();
        db.insert_usage(&usage("A", 1, 1)).unwrap();
        db.clear_usage().unwrap();
        assert!(db.usage_summary().unwrap().is_empty());
    }

    // ---------------- checkpoint ----------------

    #[test]
    fn checkpoint_succeeds() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        db.checkpoint().unwrap();
        assert!(db.formula_exists("usr:1").unwrap());
    }

    // ---------------- 序列化兼容 ----------------

    #[test]
    fn alt_expression_and_step_template_survive_roundtrip() {
        let db = test_db();
        let mut s = test_schema("usr:1");
        s.alt_expressions = vec![AltExpression {
            label: "分支".to_string(),
            expression: "a-b".to_string(),
            condition: Some("a>b".to_string()),
        }];
        s.steps_template = Some(vec![StepTemplate {
            symbol: "s1".to_string(),
            label: "第一步".to_string(),
            group: None,
            expression: "a*b".to_string(),
            unit: "m2".to_string(),
            note: None,
        }]);
        db.upsert_formula(&s).unwrap();

        let got = db.get_formula("usr:1").unwrap().unwrap();
        assert_eq!(got.alt_expressions.len(), 1);
        assert_eq!(got.alt_expressions[0].condition.as_deref(), Some("a>b"));
        assert_eq!(got.steps_template.unwrap()[0].unit, "m2");
    }

    #[test]
    fn source_kind_serializes_uppercase() {
        // 保证与源项目 Kotlin 枚举名一致
        let json = serde_json::to_string(&SourceKind::Standard).unwrap();
        assert_eq!(json, "\"STANDARD\"");
    }

    // ---------------- 用量：按天汇总 / 明细 / 计数 ----------------

    #[test]
    fn usage_summary_by_day_groups_and_orders_desc() {
        let db = test_db();
        // 直接插行以便控制 createdAt（insert_usage 用的是 now_ms）
        {
            let conn = db.lock().unwrap();
            for (label, total, created) in [
                ("deepseek", 100i64, 1_700_000_000_000i64), // 2023-11-14 UTC
                ("glm", 200, 1_700_000_000_000),
                ("deepseek", 300, 1_700_100_000_000),       // 次日
            ] {
                conn.execute(
                    "INSERT INTO llm_usage_stats
                     (modelLabel, promptTokens, completionTokens, totalTokens,
                      cachedTokens, reasoningTokens, createdAt)
                     VALUES (?1, 0, 0, ?2, 0, 0, ?3)",
                    params![label, total, created],
                )
                .unwrap();
            }
        }

        let days = db.usage_summary_by_day().unwrap();
        assert_eq!(days.len(), 2, "应分成两天");
        assert_eq!(days[0].total_all, 300, "最近的日期在前");
        assert_eq!(days[0].call_count, 1);
        assert_eq!(days[1].total_all, 300, "首日两条合计 100+200");
        assert_eq!(days[1].call_count, 2);
        // 日期是本地时区的 YYYY-MM-DD
        assert_eq!(days[0].day.len(), 10);
        assert_eq!(days[0].day.matches('-').count(), 2);
    }

    #[test]
    fn usage_summary_by_day_empty_table() {
        let db = test_db();
        assert!(db.usage_summary_by_day().unwrap().is_empty());
    }

    #[test]
    fn list_usage_is_ascending_by_time() {
        let db = test_db();
        {
            let conn = db.lock().unwrap();
            for (label, created) in [("b", 2000i64), ("a", 1000i64)] {
                conn.execute(
                    "INSERT INTO llm_usage_stats
                     (modelLabel, promptTokens, completionTokens, totalTokens,
                      cachedTokens, reasoningTokens, createdAt)
                     VALUES (?1, 1, 2, 3, 4, 5, ?2)",
                    params![label, created],
                )
                .unwrap();
            }
        }
        let list = db.list_usage().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].model_label, "a", "按时间正序");
        assert_eq!(list[1].model_label, "b");
        assert_eq!(list[0].prompt_tokens, 1);
        assert_eq!(list[0].reasoning_tokens, 5);
    }

    #[test]
    fn usage_count_tracks_insert_and_clear() {
        let db = test_db();
        assert_eq!(db.usage_count().unwrap(), 0);

        db.insert_usage(&UsageRow {
            model_label: "m".into(),
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            cached_tokens: 0,
            reasoning_tokens: 0,
        })
        .unwrap();
        assert_eq!(db.usage_count().unwrap(), 1);

        db.clear_usage().unwrap();
        assert_eq!(db.usage_count().unwrap(), 0);
    }

    /// 三张用量视图（按模型 / 按天 / 明细）的字段名是 camelCase
    #[test]
    fn usage_view_serde_is_camel_case() {
        let d = UsageDaySummary {
            day: "2026-09-17".into(),
            total_prompt: 1,
            total_completion: 2,
            total_all: 3,
            total_cached: 4,
            total_reasoning: 5,
            call_count: 6,
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["totalAll"], serde_json::json!(3));
        assert_eq!(v["callCount"], serde_json::json!(6));
        assert!(v.get("total_all").is_none(), "不得泄漏 snake_case");

        let r = UsageRecord {
            id: 1,
            model_label: "m".into(),
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            cached_tokens: 4,
            reasoning_tokens: 5,
            created_at: 6,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["modelLabel"], serde_json::json!("m"));
        assert_eq!(v["createdAt"], serde_json::json!(6));
    }

    // ---------------- 历史：按 id 取单条（回填用） ----------------

    #[test]
    fn get_history_returns_the_row() {
        let db = test_db();
        let id = db
            .insert_history(&test_history("usr:1", Some("思考")))
            .unwrap();

        let got = db.get_history(id).unwrap().expect("应取到该条");
        assert_eq!(got.id, id);
        assert_eq!(got.formula_id, "usr:1");
        assert_eq!(got.thinking_content.as_deref(), Some("思考"));
    }

    /// 不存在时返回 `Ok(None)` —— **不是错误**（历史可能刚被清掉）
    #[test]
    fn get_history_missing_is_none_not_error() {
        let db = test_db();
        assert!(db.get_history(9999).unwrap().is_none());
    }

    /// 三处 JSON 原样返回（回填要按 `inputsJson` 精确复原参数）
    #[test]
    fn get_history_preserves_raw_json() {
        let db = test_db();
        let mut e = test_history("usr:1", None);
        e.inputs_json = r#"{"a":1.5,"b":"raw"}"#.to_string();
        e.result_json = String::new(); // 空串也要原样保留
        let id = db.insert_history(&e).unwrap();

        let got = db.get_history(id).unwrap().unwrap();
        assert_eq!(got.inputs_json, r#"{"a":1.5,"b":"raw"}"#);
        assert_eq!(got.result_json, "");
    }

    #[test]
    fn get_history_after_delete_is_none() {
        let db = test_db();
        let id = db.insert_history(&test_history("usr:1", None)).unwrap();
        db.delete_history(id).unwrap();
        assert!(db.get_history(id).unwrap().is_none());
    }

    // =====================================================================
    // 备份读写（P4-14）
    // =====================================================================

    /// 🔴 收藏的 `createdAt` 必须保真
    ///
    /// `set_favorite` 会把它写成**当前时间** —— 用它来恢复备份，
    /// 「收藏于 2024 年」会变成「收藏于刚才」。
    #[test]
    fn favorite_rows_keep_original_created_at() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        db.upsert_formula(&test_schema("usr:2")).unwrap();

        for (id, ts) in [("usr:1", 1_600_000_000_000i64), ("usr:2", 1_700_000_000_000)] {
            db.upsert_favorite_row(&FavoriteTableRow {
                formula_id: id.to_string(),
                created_at: ts,
            })
            .unwrap();
        }

        let rows = db.list_favorite_rows().unwrap();
        assert_eq!(rows.len(), 2);
        // 倒序：新收藏在前
        assert_eq!(rows[0].formula_id, "usr:2");
        assert_eq!(rows[0].created_at, 1_700_000_000_000, "时间必须原样保留");
        assert_eq!(rows[1].created_at, 1_600_000_000_000);
    }

    /// 恢复收藏要**同时**把 `user_formulas.favorite` 置 1，且不动 `updatedAt`
    #[test]
    fn upsert_favorite_row_sets_flag_without_touching_updated_at() {
        let db = test_db();
        let mut s = test_schema("usr:1");
        s.created_at = 1_600_000_000_000;
        s.updated_at = 1_600_000_000_001;
        db.upsert_formula(&s).unwrap();

        db.upsert_favorite_row(&FavoriteTableRow {
            formula_id: "usr:1".to_string(),
            created_at: 1_650_000_000_000,
        })
        .unwrap();

        assert!(db.is_favorite("usr:1").unwrap(), "两处标记要一致");
        let got = db.get_formula("usr:1").unwrap().unwrap();
        assert_eq!(got.updated_at, 1_600_000_000_001, "恢复数据不是「改公式」");
        assert_eq!(got.created_at, 1_600_000_000_000);
    }

    /// 收藏行与 `list_favorite_ids` 一致（两条读路径不能打架）
    #[test]
    fn favorite_rows_agree_with_id_list() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        db.upsert_favorite_row(&FavoriteTableRow {
            formula_id: "usr:1".to_string(),
            created_at: 5,
        })
        .unwrap();

        assert_eq!(db.list_favorite_ids().unwrap(), vec!["usr:1".to_string()]);
        assert_eq!(db.list_favorite_rows().unwrap().len(), 1);
    }

    /// 跨公式的版本列表，顺序稳定（`formulaId, createdAt, version`）
    ///
    /// 源项目 `versionDao.getAll()` 没有 ORDER BY —— 顺序由查询计划决定，
    /// 会让同一次导出两次得到**不同的包字节**。
    #[test]
    fn list_all_versions_spans_formulas_in_stable_order() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        db.upsert_formula(&test_schema("usr:2")).unwrap();

        db.insert_version(&test_version_at("usr:2", "v1.0", None, 200)).unwrap();
        db.insert_version(&test_version_at("usr:1", "v2.0", Some("v1.0"), 300)).unwrap();
        db.insert_version(&test_version_at("usr:1", "v1.0", None, 100)).unwrap();

        let keys: Vec<(String, String)> = db
            .list_all_versions()
            .unwrap()
            .iter()
            .map(|v| (v.formula_id.clone(), v.version.clone()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("usr:1".to_string(), "v1.0".to_string()),
                ("usr:1".to_string(), "v2.0".to_string()),
                ("usr:2".to_string(), "v1.0".to_string()),
            ]
        );
    }

    /// 🔴 历史 id 必须保真 —— 图片的 `refs` 里有 `hist:search:<entryId>`
    #[test]
    fn insert_history_with_id_preserves_id() {
        let db = test_db();
        let mut e = test_history("usr:1", None);
        e.id = 4242;
        e.created_at = 1_600_000_000_000;

        assert_eq!(db.insert_history_with_id(&e).unwrap(), 4242);
        let got = db.get_history(4242).unwrap().unwrap();
        assert_eq!(got.created_at, 1_600_000_000_000, "时间也要保真");
        assert_eq!(got.formula_id, "usr:1");
    }

    /// `id <= 0`（未入库约定）→ 退回普通插入，由 SQLite 分配
    #[test]
    fn insert_history_with_id_falls_back_for_unassigned() {
        let db = test_db();
        let mut e = test_history("usr:1", None);
        e.id = 0;
        let id = db.insert_history_with_id(&e).unwrap();
        assert!(id > 0, "应分配到一个真实 id，实际 {id}");
    }

    /// 重复导入同一个包不产生重复行（`INSERT OR REPLACE`）
    #[test]
    fn insert_history_with_id_is_idempotent() {
        let db = test_db();
        let mut e = test_history("usr:1", None);
        e.id = 7;
        db.insert_history_with_id(&e).unwrap();
        e.thinking_content = Some("第二次".to_string());
        db.insert_history_with_id(&e).unwrap();

        assert_eq!(db.get_history(7).unwrap().unwrap().thinking_content.as_deref(), Some("第二次"));
        assert_eq!(db.list_history(None, 100, 0).unwrap().len(), 1);
    }

    /// 🔴 用量的 `createdAt` 必须保真（否则按天统计全堆到今天）
    #[test]
    fn insert_usage_at_preserves_created_at() {
        let db = test_db();
        db.insert_usage_at(&usage("m1", 10, 20), 1_500_000_000_000).unwrap();

        let rows = db.list_usage().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].created_at, 1_500_000_000_000);
        assert_eq!(rows[0].total_tokens, 30);

        // 对照：常规 `insert_usage` 写的是当前时间
        db.insert_usage(&usage("m2", 1, 1)).unwrap();
        let rows = db.list_usage().unwrap();
        let m2 = rows.iter().find(|r| r.model_label == "m2").unwrap();
        assert!(m2.created_at > 1_600_000_000_000, "常规插入写当前时间");
    }

    /// 🔴 清空公式必须连收藏与版本一起清（否则留下指向不存在公式的幽灵条目）
    #[test]
    fn clear_formulas_removes_favorites_and_versions() {
        let db = test_db();
        db.upsert_formula(&test_schema("usr:1")).unwrap();
        db.upsert_favorite_row(&FavoriteTableRow {
            formula_id: "usr:1".to_string(),
            created_at: 1,
        })
        .unwrap();
        db.insert_version(&test_version("usr:1", "v1.0", None)).unwrap();
        // 历史与用量**不属于**「公式」类别，不该被清
        let hid = db.insert_history(&test_history("usr:1", None)).unwrap();
        db.insert_usage(&usage("m1", 1, 1)).unwrap();

        db.clear_formulas().unwrap();

        assert!(db.list_formulas().unwrap().is_empty());
        assert!(db.list_favorite_rows().unwrap().is_empty(), "收藏要一起清");
        assert!(db.list_all_versions().unwrap().is_empty(), "版本要一起清");
        assert!(db.get_history(hid).unwrap().is_some(), "历史不归公式类别管");
        assert_eq!(db.usage_count().unwrap(), 1, "用量不归公式类别管");
    }

    /// 空库上清空不报错（幂等）
    #[test]
    fn clear_formulas_on_empty_db_is_ok() {
        let db = test_db();
        db.clear_formulas().unwrap();
        db.clear_formulas().unwrap();
        assert!(db.list_formulas().unwrap().is_empty());
    }
}
