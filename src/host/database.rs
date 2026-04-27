use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock};

/// Database access mode used by one host-facing runtime backend.
/// 单个宿主侧运行时后端所使用的数据库访问模式。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LuaRuntimeDatabaseProviderMode {
    /// The library loads and calls the local dynamic-library backend directly.
    /// 由库直接加载并调用本地动态库后端。
    #[default]
    DynamicLibrary,
    /// The library forwards database operations into one host-registered callback bridge.
    /// 由库把数据库操作转发给宿主已注册的回调桥接。
    HostCallback,
    /// The library forwards database operations into one external space controller.
    /// 由库把数据库操作转发给外部空间控制器。
    SpaceController,
}

/// Callback transport mode used when the database provider mode is `host_callback`.
/// 当数据库 provider 模式为 `host_callback` 时所使用的回调传输模式。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LuaRuntimeDatabaseCallbackMode {
    /// The library uses the structured standard callback ABI.
    /// 由库使用结构化标准回调 ABI。
    #[default]
    Standard,
    /// The library uses the JSON callback ABI.
    /// 由库使用 JSON 回调 ABI。
    Json,
}

/// Logical database kind resolved for one provider request.
/// 为单次 provider 请求解析出的逻辑数据库类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDatabaseKind {
    /// SQLite / FTS / BM25 backend operations.
    /// SQLite / FTS / BM25 后端操作。
    Sqlite,
    /// LanceDB vector backend operations.
    /// LanceDB 向量后端操作。
    LanceDb,
}

/// Stable host-facing binding context for one skill-scoped database backend.
/// 面向宿主的稳定 skill 级数据库后端绑定上下文。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeDatabaseBindingContext {
    /// Stable root label supplied by the host such as ROOT, PROJECT, or USER.
    /// 由宿主提供的稳定根标签，例如 ROOT、PROJECT 或 USER。
    pub space_label: String,
    /// Stable skill identifier currently owning the database binding.
    /// 当前拥有数据库绑定的稳定技能标识符。
    pub skill_id: String,
    /// Stable binding tag composed from the space label and skill id.
    /// 由空间标签与技能标识符组合得到的稳定绑定标签。
    pub binding_tag: String,
    /// Physical skill root label currently resolving the effective skill instance.
    /// 当前解析出生效技能实例时所命中的物理技能根标签。
    pub root_name: String,
    /// Physical skill root directory that owns the current effective skill instance.
    /// 当前生效技能实例所属的物理技能根目录。
    pub space_root: String,
    /// Physical skill directory path.
    /// 物理技能目录路径。
    pub skill_dir: String,
    /// Physical skill directory basename.
    /// 物理技能目录名称。
    pub skill_dir_name: String,
    /// Logical database kind requested by the current provider binding.
    /// 当前 provider 绑定请求的逻辑数据库类型。
    pub database_kind: RuntimeDatabaseKind,
    /// Default embedded database path resolved by the library for compatibility and diagnostics.
    /// 由库按内嵌规则解析出的默认数据库路径，用于兼容和诊断。
    pub default_database_path: String,
}

impl RuntimeDatabaseBindingContext {
    /// Build one stable binding context from host-resolved root and skill information.
    /// 基于宿主已解析的根信息与技能信息构造稳定绑定上下文。
    pub fn new(
        space_label: impl Into<String>,
        skill_id: impl Into<String>,
        root_name: impl Into<String>,
        space_root: impl Into<String>,
        skill_dir: impl Into<String>,
        skill_dir_name: impl Into<String>,
        database_kind: RuntimeDatabaseKind,
        default_database_path: impl Into<String>,
    ) -> Self {
        let space_label = space_label.into();
        let skill_id = skill_id.into();
        Self {
            binding_tag: format!("{}-{}", space_label, skill_id),
            space_label,
            skill_id,
            root_name: root_name.into(),
            space_root: space_root.into(),
            skill_dir: skill_dir.into(),
            skill_dir_name: skill_dir_name.into(),
            database_kind,
            default_database_path: default_database_path.into(),
        }
    }
}

/// Structured SQLite provider action routed through one host bridge.
/// 通过宿主桥接路由的结构化 SQLite provider 动作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSqliteProviderAction {
    /// Execute one SQL script or one single SQL statement.
    /// 执行一个 SQL 脚本或单条 SQL 语句。
    ExecuteScript,
    /// Execute one batch SQL write request.
    /// 执行一次批量 SQL 写入请求。
    ExecuteBatch,
    /// Execute one JSON row-set query.
    /// 执行一次 JSON 行集查询。
    QueryJson,
    /// Create one query-stream handle.
    /// 创建一个查询流句柄。
    QueryStream,
    /// Wait for query-stream metrics.
    /// 等待查询流统计信息。
    QueryStreamWaitMetrics,
    /// Read one query-stream chunk.
    /// 读取一个查询流分块。
    QueryStreamChunk,
    /// Close one query-stream handle.
    /// 关闭一个查询流句柄。
    QueryStreamClose,
    /// Execute text tokenization.
    /// 执行文本分词。
    TokenizeText,
    /// Upsert one custom dictionary word.
    /// 写入或更新一个自定义词。
    UpsertCustomWord,
    /// Remove one custom dictionary word.
    /// 删除一个自定义词。
    RemoveCustomWord,
    /// List current custom dictionary words.
    /// 列出当前自定义词。
    ListCustomWords,
    /// Ensure one FTS index exists.
    /// 确保一个 FTS 索引存在。
    EnsureFtsIndex,
    /// Rebuild one FTS index.
    /// 重建一个 FTS 索引。
    RebuildFtsIndex,
    /// Upsert one FTS document.
    /// 写入或更新一条 FTS 文档。
    UpsertFtsDocument,
    /// Delete one FTS document.
    /// 删除一条 FTS 文档。
    DeleteFtsDocument,
    /// Execute one standardized FTS/BM25 search.
    /// 执行一次标准化 FTS/BM25 检索。
    SearchFts,
}

/// Structured LanceDB provider action routed through one host bridge.
/// 通过宿主桥接路由的结构化 LanceDB provider 动作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLanceDbProviderAction {
    /// Create one table.
    /// 创建一张表。
    CreateTable,
    /// Upsert vectors into one table.
    /// 向一张表写入向量。
    VectorUpsert,
    /// Search vectors from one table.
    /// 从一张表检索向量。
    VectorSearch,
    /// Delete rows from one table.
    /// 从一张表删除行。
    Delete,
    /// Drop one table.
    /// 删除一张表。
    DropTable,
}

/// Structured SQLite provider request delivered to one host bridge.
/// 传递给宿主桥接的结构化 SQLite provider 请求。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSqliteProviderRequest {
    /// Requested SQLite provider action.
    /// 请求的 SQLite provider 动作。
    pub action: RuntimeSqliteProviderAction,
    /// Stable binding context of the current skill-scoped database.
    /// 当前 skill 级数据库的稳定绑定上下文。
    pub binding: RuntimeDatabaseBindingContext,
    /// Action-specific JSON input payload.
    /// 动作对应的 JSON 输入载荷。
    pub input: Value,
}

/// Structured LanceDB provider request delivered to one host bridge.
/// 传递给宿主桥接的结构化 LanceDB provider 请求。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeLanceDbProviderRequest {
    /// Requested LanceDB provider action.
    /// 请求的 LanceDB provider 动作。
    pub action: RuntimeLanceDbProviderAction,
    /// Stable binding context of the current skill-scoped database.
    /// 当前 skill 级数据库的稳定绑定上下文。
    pub binding: RuntimeDatabaseBindingContext,
    /// Action-specific JSON input payload.
    /// 动作对应的 JSON 输入载荷。
    pub input: Value,
}

/// Standard host callback used for one structured SQLite provider request.
/// 用于处理结构化 SQLite provider 请求的标准宿主回调。
pub type RuntimeSqliteProviderCallback =
    Arc<dyn Fn(&RuntimeSqliteProviderRequest) -> Result<Value, String> + Send + Sync>;

/// Standard host callback used for one structured LanceDB provider request.
/// 用于处理结构化 LanceDB provider 请求的标准宿主回调。
pub type RuntimeLanceDbProviderCallback = Arc<
    dyn Fn(&RuntimeLanceDbProviderRequest) -> Result<RuntimeLanceDbProviderResult, String>
        + Send
        + Sync,
>;

/// JSON host callback used for one SQLite provider request.
/// 用于处理 SQLite provider 请求的 JSON 宿主回调。
pub type RuntimeSqliteProviderJsonCallback =
    Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// JSON host callback used for one LanceDB provider request.
/// 用于处理 LanceDB provider 请求的 JSON 宿主回调。
pub type RuntimeLanceDbProviderJsonCallback =
    Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// One engine-scoped snapshot of all database provider callbacks visible at creation time.
/// 一个在引擎创建时快照出的数据库 provider 回调集合，作用域限定为单个引擎实例。
#[derive(Clone, Default)]
pub(crate) struct RuntimeDatabaseProviderCallbacks {
    /// Structured SQLite callback captured for the current engine snapshot.
    /// 当前引擎快照捕获到的结构化 SQLite 回调。
    sqlite_standard: Option<RuntimeSqliteProviderCallback>,
    /// Structured LanceDB callback captured for the current engine snapshot.
    /// 当前引擎快照捕获到的结构化 LanceDB 回调。
    lancedb_standard: Option<RuntimeLanceDbProviderCallback>,
    /// JSON SQLite callback captured for the current engine snapshot.
    /// 当前引擎快照捕获到的 JSON SQLite 回调。
    sqlite_json: Option<RuntimeSqliteProviderJsonCallback>,
    /// JSON LanceDB callback captured for the current engine snapshot.
    /// 当前引擎快照捕获到的 JSON LanceDB 回调。
    lancedb_json: Option<RuntimeLanceDbProviderJsonCallback>,
}

impl RuntimeDatabaseProviderCallbacks {
    /// Snapshot the current process-wide callback defaults into one engine-private registry.
    /// 把当前进程级默认回调快照为一个引擎私有注册表。
    pub(crate) fn capture_process_defaults() -> Result<Self, String> {
        Ok(Self {
            sqlite_standard: take_optional_callback(sqlite_provider_callback_registry())?,
            lancedb_standard: take_optional_callback(lancedb_provider_callback_registry())?,
            sqlite_json: take_optional_callback(sqlite_provider_json_callback_registry())?,
            lancedb_json: take_optional_callback(lancedb_provider_json_callback_registry())?,
        })
    }

    /// Return whether the snapshot contains one SQLite callback for the requested transport mode.
    /// 返回当前快照是否包含指定传输模式的 SQLite 回调。
    pub(crate) fn has_sqlite_provider_callback_for_mode(
        &self,
        callback_mode: LuaRuntimeDatabaseCallbackMode,
    ) -> bool {
        match callback_mode {
            LuaRuntimeDatabaseCallbackMode::Standard => self.sqlite_standard.is_some(),
            LuaRuntimeDatabaseCallbackMode::Json => self.sqlite_json.is_some(),
        }
    }

    /// Return whether the snapshot contains one LanceDB callback for the requested transport mode.
    /// 返回当前快照是否包含指定传输模式的 LanceDB 回调。
    pub(crate) fn has_lancedb_provider_callback_for_mode(
        &self,
        callback_mode: LuaRuntimeDatabaseCallbackMode,
    ) -> bool {
        match callback_mode {
            LuaRuntimeDatabaseCallbackMode::Standard => self.lancedb_standard.is_some(),
            LuaRuntimeDatabaseCallbackMode::Json => self.lancedb_json.is_some(),
        }
    }

    /// Dispatch one SQLite provider request through the callbacks captured by this snapshot.
    /// 通过当前快照捕获的回调分发一次 SQLite provider 请求。
    pub(crate) fn dispatch_sqlite_provider_request(
        &self,
        request: &RuntimeSqliteProviderRequest,
        callback_mode: LuaRuntimeDatabaseCallbackMode,
    ) -> Result<Value, String> {
        match callback_mode {
            LuaRuntimeDatabaseCallbackMode::Standard => {
                let callback = self.sqlite_standard.clone().ok_or_else(|| {
                    "SQLite host-callback mode requires one registered standard callback"
                        .to_string()
                })?;
                callback(request)
            }
            LuaRuntimeDatabaseCallbackMode::Json => {
                let callback = self.sqlite_json.clone().ok_or_else(|| {
                    "SQLite host-callback JSON mode requires one registered JSON callback"
                        .to_string()
                })?;
                let request_json = serde_json::to_string(request).map_err(|error| {
                    format!("failed to encode sqlite provider request: {}", error)
                })?;
                let response_json = callback(&request_json)?;
                serde_json::from_str::<Value>(&response_json).map_err(|error| {
                    format!("failed to parse sqlite provider response json: {}", error)
                })
            }
        }
    }

    /// Dispatch one LanceDB provider request through the callbacks captured by this snapshot.
    /// 通过当前快照捕获的回调分发一次 LanceDB provider 请求。
    pub(crate) fn dispatch_lancedb_provider_request(
        &self,
        request: &RuntimeLanceDbProviderRequest,
        callback_mode: LuaRuntimeDatabaseCallbackMode,
    ) -> Result<RuntimeLanceDbProviderResult, String> {
        match callback_mode {
            LuaRuntimeDatabaseCallbackMode::Standard => {
                let callback = self.lancedb_standard.clone().ok_or_else(|| {
                    "LanceDB host-callback mode requires one registered standard callback"
                        .to_string()
                })?;
                callback(request)
            }
            LuaRuntimeDatabaseCallbackMode::Json => {
                let callback = self.lancedb_json.clone().ok_or_else(|| {
                    "LanceDB host-callback JSON mode requires one registered JSON callback"
                        .to_string()
                })?;
                let request_json = serde_json::to_string(request).map_err(|error| {
                    format!("failed to encode lancedb provider request: {}", error)
                })?;
                let response_json = callback(&request_json)?;
                let value: Value = serde_json::from_str(&response_json).map_err(|error| {
                    format!("failed to parse lancedb provider response json: {}", error)
                })?;
                let meta = value
                    .get("meta")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Default::default()));
                let bytes = value
                    .get("data_base64")
                    .and_then(Value::as_str)
                    .map(|text| {
                        BASE64_STANDARD.decode(text.as_bytes()).map_err(|error| {
                            format!("failed to decode lancedb provider data_base64: {}", error)
                        })
                    })
                    .transpose()?
                    .unwrap_or_default();
                Ok(RuntimeLanceDbProviderResult::binary(meta, bytes))
            }
        }
    }
}

/// Structured LanceDB provider result returned by the standard host callback.
/// 标准宿主回调返回的结构化 LanceDB provider 结果。
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeLanceDbProviderResult {
    /// Response metadata JSON.
    /// 响应元信息 JSON。
    pub meta: Value,
    /// Optional raw payload bytes such as vector search result data.
    /// 可选原始载荷字节，例如向量检索结果数据。
    pub bytes: Vec<u8>,
}

impl RuntimeLanceDbProviderResult {
    /// Build one result carrying only metadata JSON.
    /// 构造一个仅携带元信息 JSON 的结果。
    pub fn json(meta: Value) -> Self {
        Self {
            meta,
            bytes: Vec::new(),
        }
    }

    /// Build one result carrying metadata JSON plus raw bytes.
    /// 构造一个携带元信息 JSON 和原始字节的结果。
    pub fn binary(meta: Value, bytes: Vec<u8>) -> Self {
        Self { meta, bytes }
    }
}

/// Install or clear the process-wide standard SQLite provider callback.
/// 安装或清理进程级标准 SQLite provider 回调。
pub fn set_sqlite_provider_callback(callback: Option<RuntimeSqliteProviderCallback>) {
    let registry = sqlite_provider_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Install or clear the process-wide standard LanceDB provider callback.
/// 安装或清理进程级标准 LanceDB provider 回调。
pub fn set_lancedb_provider_callback(callback: Option<RuntimeLanceDbProviderCallback>) {
    let registry = lancedb_provider_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Install or clear the process-wide JSON SQLite provider callback.
/// 安装或清理进程级 JSON SQLite provider 回调。
pub fn set_sqlite_provider_json_callback(callback: Option<RuntimeSqliteProviderJsonCallback>) {
    let registry = sqlite_provider_json_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Install or clear the process-wide JSON LanceDB provider callback.
/// 安装或清理进程级 JSON LanceDB provider 回调。
pub fn set_lancedb_provider_json_callback(callback: Option<RuntimeLanceDbProviderJsonCallback>) {
    let registry = lancedb_provider_json_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Read one optional callback from one mutex registry without cloning error-prone lock code at each call site.
/// 从一个互斥量注册表读取可选回调，避免在每个调用点重复编写易错的加锁逻辑。
fn take_optional_callback<T: Clone>(
    registry: &'static Mutex<Option<T>>,
) -> Result<Option<T>, String> {
    let guard = registry
        .lock()
        .map_err(|_| "Database provider callback registry lock poisoned".to_string())?;
    Ok(guard.clone())
}

/// Return the process-wide standard SQLite provider callback storage.
/// 返回进程级标准 SQLite provider 回调存储。
fn sqlite_provider_callback_registry() -> &'static Mutex<Option<RuntimeSqliteProviderCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeSqliteProviderCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Return the process-wide standard LanceDB provider callback storage.
/// 返回进程级标准 LanceDB provider 回调存储。
fn lancedb_provider_callback_registry() -> &'static Mutex<Option<RuntimeLanceDbProviderCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeLanceDbProviderCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Return the process-wide JSON SQLite provider callback storage.
/// 返回进程级 JSON SQLite provider 回调存储。
fn sqlite_provider_json_callback_registry()
-> &'static Mutex<Option<RuntimeSqliteProviderJsonCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeSqliteProviderJsonCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Return the process-wide JSON LanceDB provider callback storage.
/// 返回进程级 JSON LanceDB provider 回调存储。
fn lancedb_provider_json_callback_registry()
-> &'static Mutex<Option<RuntimeLanceDbProviderJsonCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeLanceDbProviderJsonCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};

    /// Return a process-wide test lock so callback-registry tests do not race in parallel.
    /// 返回一个进程级测试锁，避免回调注册表测试并发互相干扰。
    fn database_callback_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Restore the process-wide callback defaults captured before one test mutates them.
    /// 恢复某个测试修改前捕获到的进程级默认回调集合。
    struct ProcessCallbackRestoreGuard {
        snapshot: RuntimeDatabaseProviderCallbacks,
    }

    impl ProcessCallbackRestoreGuard {
        /// Capture the current process-wide callback defaults so they can be restored on drop.
        /// 捕获当前进程级默认回调，以便在释放时恢复。
        fn capture() -> Self {
            Self {
                snapshot: RuntimeDatabaseProviderCallbacks::capture_process_defaults()
                    .expect("capture callback snapshot"),
            }
        }
    }

    impl Drop for ProcessCallbackRestoreGuard {
        fn drop(&mut self) {
            set_sqlite_provider_callback(self.snapshot.sqlite_standard.clone());
            set_lancedb_provider_callback(self.snapshot.lancedb_standard.clone());
            set_sqlite_provider_json_callback(self.snapshot.sqlite_json.clone());
            set_lancedb_provider_json_callback(self.snapshot.lancedb_json.clone());
        }
    }

    /// Build one stable binding context used by snapshot-isolation tests.
    /// 构造供快照隔离测试使用的稳定绑定上下文。
    fn sample_binding_context(database_kind: RuntimeDatabaseKind) -> RuntimeDatabaseBindingContext {
        RuntimeDatabaseBindingContext::new(
            "ROOT",
            "test-skill",
            "ROOT",
            "D:/runtime-test-root/__database",
            "D:/runtime-test-root/skills/test-skill",
            "test-skill",
            database_kind,
            "D:/runtime-test-root/__database/default.db",
        )
    }

    /// Verify that each captured callback snapshot keeps routing to the callbacks visible at capture time.
    /// 验证每个捕获到的回调快照都会持续路由到捕获当时可见的回调实现。
    #[test]
    fn captured_callback_snapshots_stay_engine_scoped() {
        let _serial_guard = database_callback_test_lock()
            .lock()
            .expect("lock callback test guard");
        let _restore_guard = ProcessCallbackRestoreGuard::capture();

        set_sqlite_provider_callback(Some(Arc::new(|_| {
            Ok(json!({ "source": "sqlite-standard-a" }))
        })));
        set_sqlite_provider_json_callback(Some(Arc::new(|_| {
            Ok("{\"source\":\"sqlite-json-a\"}".to_string())
        })));
        set_lancedb_provider_callback(Some(Arc::new(|_| {
            Ok(RuntimeLanceDbProviderResult::json(
                json!({ "source": "lancedb-standard-a" }),
            ))
        })));
        set_lancedb_provider_json_callback(Some(Arc::new(|_| {
            Ok("{\"meta\":{\"source\":\"lancedb-json-a\"}}".to_string())
        })));
        let snapshot_a = RuntimeDatabaseProviderCallbacks::capture_process_defaults()
            .expect("capture callback snapshot A");

        set_sqlite_provider_callback(Some(Arc::new(|_| {
            Ok(json!({ "source": "sqlite-standard-b" }))
        })));
        set_sqlite_provider_json_callback(Some(Arc::new(|_| {
            Ok("{\"source\":\"sqlite-json-b\"}".to_string())
        })));
        set_lancedb_provider_callback(Some(Arc::new(|_| {
            Ok(RuntimeLanceDbProviderResult::json(
                json!({ "source": "lancedb-standard-b" }),
            ))
        })));
        set_lancedb_provider_json_callback(Some(Arc::new(|_| {
            Ok("{\"meta\":{\"source\":\"lancedb-json-b\"}}".to_string())
        })));
        let snapshot_b = RuntimeDatabaseProviderCallbacks::capture_process_defaults()
            .expect("capture callback snapshot B");

        let sqlite_request = RuntimeSqliteProviderRequest {
            action: RuntimeSqliteProviderAction::QueryJson,
            binding: sample_binding_context(RuntimeDatabaseKind::Sqlite),
            input: json!({ "sql": "select 1" }),
        };
        let lancedb_request = RuntimeLanceDbProviderRequest {
            action: RuntimeLanceDbProviderAction::VectorSearch,
            binding: sample_binding_context(RuntimeDatabaseKind::LanceDb),
            input: json!({ "table": "demo" }),
        };

        assert_eq!(
            snapshot_a
                .dispatch_sqlite_provider_request(
                    &sqlite_request,
                    LuaRuntimeDatabaseCallbackMode::Standard,
                )
                .expect("dispatch sqlite standard A"),
            json!({ "source": "sqlite-standard-a" })
        );
        assert_eq!(
            snapshot_a
                .dispatch_sqlite_provider_request(
                    &sqlite_request,
                    LuaRuntimeDatabaseCallbackMode::Json,
                )
                .expect("dispatch sqlite json A"),
            json!({ "source": "sqlite-json-a" })
        );
        assert_eq!(
            snapshot_b
                .dispatch_sqlite_provider_request(
                    &sqlite_request,
                    LuaRuntimeDatabaseCallbackMode::Standard,
                )
                .expect("dispatch sqlite standard B"),
            json!({ "source": "sqlite-standard-b" })
        );
        assert_eq!(
            snapshot_b
                .dispatch_sqlite_provider_request(
                    &sqlite_request,
                    LuaRuntimeDatabaseCallbackMode::Json,
                )
                .expect("dispatch sqlite json B"),
            json!({ "source": "sqlite-json-b" })
        );

        assert_eq!(
            snapshot_a
                .dispatch_lancedb_provider_request(
                    &lancedb_request,
                    LuaRuntimeDatabaseCallbackMode::Standard,
                )
                .expect("dispatch lancedb standard A")
                .meta,
            json!({ "source": "lancedb-standard-a" })
        );
        assert_eq!(
            snapshot_a
                .dispatch_lancedb_provider_request(
                    &lancedb_request,
                    LuaRuntimeDatabaseCallbackMode::Json,
                )
                .expect("dispatch lancedb json A")
                .meta,
            json!({ "source": "lancedb-json-a" })
        );
        assert_eq!(
            snapshot_b
                .dispatch_lancedb_provider_request(
                    &lancedb_request,
                    LuaRuntimeDatabaseCallbackMode::Standard,
                )
                .expect("dispatch lancedb standard B")
                .meta,
            json!({ "source": "lancedb-standard-b" })
        );
        assert_eq!(
            snapshot_b
                .dispatch_lancedb_provider_request(
                    &lancedb_request,
                    LuaRuntimeDatabaseCallbackMode::Json,
                )
                .expect("dispatch lancedb json B")
                .meta,
            json!({ "source": "lancedb-json-b" })
        );
    }
}
