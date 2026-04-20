use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use libloading::Library;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::lua_skill::{SkillSqliteLogLevel, SkillSqliteMeta};
use crate::runtime_logging::{info as log_info, warn as log_warn};
use crate::runtime_options::LuaRuntimeHostOptions;

/// Forward declaration of the FFI runtime handle used only for raw cross-library pointers.
/// FFI runtime 句柄前置声明，仅用于跨动态库传递裸指针。
#[repr(C)]
struct VldbSqliteRuntimeHandle {
    _private: [u8; 0],
}

/// Forward declaration of the FFI database handle used only for raw cross-library pointers.
/// FFI 数据库句柄前置声明，仅用于跨动态库传递裸指针。
#[repr(C)]
struct VldbSqliteDatabaseHandle {
    _private: [u8; 0],
}

/// Forward declaration of the FFI tokenize-result handle.
/// FFI 分词结果句柄前置声明。
#[repr(C)]
struct VldbSqliteTokenizeResultHandle {
    _private: [u8; 0],
}

/// Forward declaration of the FFI custom-word list handle.
/// FFI 自定义词列表句柄前置声明。
#[repr(C)]
struct VldbSqliteCustomWordListHandle {
    _private: [u8; 0],
}

/// Forward declaration of the FFI search-result handle.
/// FFI 检索结果句柄前置声明。
#[repr(C)]
struct VldbSqliteSearchResultHandle {
    _private: [u8; 0],
}

/// Forward declaration of the FFI shared SQL execute-result handle.
/// FFI 通用 SQL 执行结果句柄前置声明。
#[repr(C)]
struct VldbSqliteExecuteResultHandle {
    _private: [u8; 0],
}

/// Forward declaration of the FFI JSON-query result handle.
/// FFI JSON 查询结果句柄前置声明。
#[repr(C)]
struct VldbSqliteQueryJsonResultHandle {
    _private: [u8; 0],
}

/// Forward declaration of the FFI QueryStream result handle.
/// FFI QueryStream 结果句柄前置声明。
#[repr(C)]
struct VldbSqliteQueryStreamHandle {
    _private: [u8; 0],
}

/// SQLite FFI status code.
/// SQLite FFI 返回状态码。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VldbSqliteStatusCode {
    Success = 0,
}

/// SQLite FFI tokenizer-mode enum kept ABI-compatible with the exported header.
/// SQLite FFI 分词模式枚举，需与导出头文件严格保持一致。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VldbSqliteFfiTokenizerMode {
    None = 0,
    Jieba = 1,
}

/// POD result structure for custom-word mutations.
/// 自定义词修改结果 POD 结构。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct VldbSqliteDictionaryMutationResultPod {
    success: u8,
    affected_rows: u64,
}

/// POD result structure for FTS ensure-index operations.
/// FTS 索引创建结果 POD 结构。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct VldbSqliteEnsureFtsIndexResultPod {
    success: u8,
    tokenizer_mode: u32,
}

/// POD result structure for FTS rebuild-index operations.
/// FTS 索引重建结果 POD 结构。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct VldbSqliteRebuildFtsIndexResultPod {
    success: u8,
    tokenizer_mode: u32,
    reindexed_rows: u64,
}

/// POD result structure for FTS document mutations.
/// FTS 文档写入/删除结果 POD 结构。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct VldbSqliteFtsMutationResultPod {
    success: u8,
    affected_rows: u64,
}

/// FFI byte-view structure used for bytes parameters.
/// FFI 字节视图结构，供 bytes 参数使用。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct VldbSqliteByteView {
    data: *const u8,
    len: u64,
}

/// FFI releasable byte buffer returned by QueryStream chunk getters.
/// FFI 可释放字节缓冲区，供 QueryStream chunk getter 返回。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct VldbSqliteByteBuffer {
    data: *mut u8,
    len: u64,
    cap: u64,
}

/// FFI SQL value-kind enum kept ABI-compatible with the exported header.
/// FFI SQL 值类型枚举，必须与头文件定义保持一致。
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VldbSqliteFfiValueKind {
    Null = 0,
    Int64 = 1,
    Float64 = 2,
    String = 3,
    Bytes = 4,
    Bool = 5,
}

/// FFI SQL parameter value structure.
/// FFI SQL 参数值结构。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct VldbSqliteFfiValue {
    kind: VldbSqliteFfiValueKind,
    int64_value: i64,
    float64_value: f64,
    string_value: *const c_char,
    bytes_value: VldbSqliteByteView,
    bool_value: u8,
}

/// FFI SQL parameter-slice structure used by batch execution.
/// FFI SQL 参数切片结构，用于批量执行。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct VldbSqliteFfiValueSlice {
    values: *const VldbSqliteFfiValue,
    len: u64,
}

type RuntimeCreateDefaultFn = unsafe extern "C" fn() -> *mut VldbSqliteRuntimeHandle;
type RuntimeDestroyFn = unsafe extern "C" fn(*mut VldbSqliteRuntimeHandle);
type RuntimeOpenDatabaseFn = unsafe extern "C" fn(
    *mut VldbSqliteRuntimeHandle,
    *const c_char,
) -> *mut VldbSqliteDatabaseHandle;
type DatabaseDestroyFn = unsafe extern "C" fn(*mut VldbSqliteDatabaseHandle);
type DatabaseDbPathFn = unsafe extern "C" fn(*mut VldbSqliteDatabaseHandle) -> *mut c_char;
type StringFreeFn = unsafe extern "C" fn(*mut c_char);
type LastErrorMessageFn = unsafe extern "C" fn() -> *const c_char;
type ClearLastErrorFn = unsafe extern "C" fn();
type LibraryInfoJsonFn = unsafe extern "C" fn() -> *mut c_char;
type DatabaseExecuteScriptFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    *const VldbSqliteFfiValue,
    u64,
    *const c_char,
) -> *mut VldbSqliteExecuteResultHandle;
type DatabaseExecuteBatchFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    *const VldbSqliteFfiValueSlice,
    u64,
) -> *mut VldbSqliteExecuteResultHandle;
type DatabaseQueryJsonFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    *const VldbSqliteFfiValue,
    u64,
    *const c_char,
) -> *mut VldbSqliteQueryJsonResultHandle;
type DatabaseQueryStreamFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    *const VldbSqliteFfiValue,
    u64,
    *const c_char,
    u64,
) -> *mut VldbSqliteQueryStreamHandle;
type ExecuteResultDestroyFn = unsafe extern "C" fn(*mut VldbSqliteExecuteResultHandle);
type ExecuteResultSuccessFn = unsafe extern "C" fn(*mut VldbSqliteExecuteResultHandle) -> u8;
type ExecuteResultMessageFn =
    unsafe extern "C" fn(*mut VldbSqliteExecuteResultHandle) -> *mut c_char;
type ExecuteResultRowsChangedFn = unsafe extern "C" fn(*mut VldbSqliteExecuteResultHandle) -> i64;
type ExecuteResultLastInsertRowIdFn =
    unsafe extern "C" fn(*mut VldbSqliteExecuteResultHandle) -> i64;
type ExecuteResultStatementsExecutedFn =
    unsafe extern "C" fn(*mut VldbSqliteExecuteResultHandle) -> i64;
type QueryJsonResultDestroyFn = unsafe extern "C" fn(*mut VldbSqliteQueryJsonResultHandle);
type QueryJsonResultJsonDataFn =
    unsafe extern "C" fn(*mut VldbSqliteQueryJsonResultHandle) -> *mut c_char;
type QueryJsonResultRowCountFn = unsafe extern "C" fn(*mut VldbSqliteQueryJsonResultHandle) -> u64;
type QueryStreamDestroyFn = unsafe extern "C" fn(*mut VldbSqliteQueryStreamHandle);
type QueryStreamChunkCountFn = unsafe extern "C" fn(*mut VldbSqliteQueryStreamHandle) -> u64;
type QueryStreamRowCountFn = unsafe extern "C" fn(*mut VldbSqliteQueryStreamHandle) -> u64;
type QueryStreamTotalBytesFn = unsafe extern "C" fn(*mut VldbSqliteQueryStreamHandle) -> u64;
type QueryStreamGetChunkFn =
    unsafe extern "C" fn(*mut VldbSqliteQueryStreamHandle, u64) -> VldbSqliteByteBuffer;
type BytesFreeFn = unsafe extern "C" fn(VldbSqliteByteBuffer);
type DatabaseTokenizeTextFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    VldbSqliteFfiTokenizerMode,
    *const c_char,
    u8,
) -> *mut VldbSqliteTokenizeResultHandle;
type TokenizeResultDestroyFn = unsafe extern "C" fn(*mut VldbSqliteTokenizeResultHandle);
type TokenizeResultNormalizedTextFn =
    unsafe extern "C" fn(*mut VldbSqliteTokenizeResultHandle) -> *mut c_char;
type TokenizeResultFtsQueryFn =
    unsafe extern "C" fn(*mut VldbSqliteTokenizeResultHandle) -> *mut c_char;
type TokenizeResultTokenCountFn = unsafe extern "C" fn(*mut VldbSqliteTokenizeResultHandle) -> u64;
type TokenizeResultGetTokenFn =
    unsafe extern "C" fn(*mut VldbSqliteTokenizeResultHandle, u64) -> *mut c_char;
type DatabaseUpsertCustomWordFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    u64,
    *mut VldbSqliteDictionaryMutationResultPod,
) -> i32;
type DatabaseRemoveCustomWordFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    *mut VldbSqliteDictionaryMutationResultPod,
) -> i32;
type DatabaseListCustomWordsFn =
    unsafe extern "C" fn(*mut VldbSqliteDatabaseHandle) -> *mut VldbSqliteCustomWordListHandle;
type CustomWordListDestroyFn = unsafe extern "C" fn(*mut VldbSqliteCustomWordListHandle);
type CustomWordListLenFn = unsafe extern "C" fn(*mut VldbSqliteCustomWordListHandle) -> u64;
type CustomWordListGetWordFn =
    unsafe extern "C" fn(*mut VldbSqliteCustomWordListHandle, u64) -> *mut c_char;
type CustomWordListGetWeightFn =
    unsafe extern "C" fn(*mut VldbSqliteCustomWordListHandle, u64) -> u64;
type DatabaseEnsureFtsIndexFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    VldbSqliteFfiTokenizerMode,
    *mut VldbSqliteEnsureFtsIndexResultPod,
) -> i32;
type DatabaseRebuildFtsIndexFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    VldbSqliteFfiTokenizerMode,
    *mut VldbSqliteRebuildFtsIndexResultPod,
) -> i32;
type DatabaseUpsertFtsDocumentFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    VldbSqliteFfiTokenizerMode,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
    *mut VldbSqliteFtsMutationResultPod,
) -> i32;
type DatabaseDeleteFtsDocumentFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    *const c_char,
    *mut VldbSqliteFtsMutationResultPod,
) -> i32;
type DatabaseSearchFtsFn = unsafe extern "C" fn(
    *mut VldbSqliteDatabaseHandle,
    *const c_char,
    VldbSqliteFfiTokenizerMode,
    *const c_char,
    u32,
    u32,
) -> *mut VldbSqliteSearchResultHandle;
type SearchResultDestroyFn = unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle);
type SearchResultTotalFn = unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle) -> u64;
type SearchResultLenFn = unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle) -> u64;
type SearchResultSourceFn = unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle) -> *mut c_char;
type SearchResultQueryModeFn =
    unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle) -> *mut c_char;
type SearchResultGetIdFn =
    unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle, u64) -> *mut c_char;
type SearchResultGetFilePathFn =
    unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle, u64) -> *mut c_char;
type SearchResultGetTitleFn =
    unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle, u64) -> *mut c_char;
type SearchResultGetTitleHighlightFn =
    unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle, u64) -> *mut c_char;
type SearchResultGetContentSnippetFn =
    unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle, u64) -> *mut c_char;
type SearchResultGetScoreFn = unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle, u64) -> f64;
type SearchResultGetRankFn = unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle, u64) -> u64;
type SearchResultGetRawScoreFn =
    unsafe extern "C" fn(*mut VldbSqliteSearchResultHandle, u64) -> f64;

/// Loaded SQLite FFI API table that owns the dynamic-library lifetime and all exported function pointers.
/// 已加载的 SQLite FFI API 表，持有动态库生命周期与全部导出函数指针。
struct LoadedSqliteApi {
    _library: Library,
    library_path: PathBuf,
    runtime_create_default: RuntimeCreateDefaultFn,
    runtime_destroy: RuntimeDestroyFn,
    runtime_open_database: RuntimeOpenDatabaseFn,
    database_destroy: DatabaseDestroyFn,
    database_db_path: DatabaseDbPathFn,
    string_free: StringFreeFn,
    last_error_message: LastErrorMessageFn,
    clear_last_error: ClearLastErrorFn,
    library_info_json: LibraryInfoJsonFn,
    database_execute_script: DatabaseExecuteScriptFn,
    database_execute_batch: DatabaseExecuteBatchFn,
    database_query_json: DatabaseQueryJsonFn,
    database_query_stream: DatabaseQueryStreamFn,
    execute_result_destroy: ExecuteResultDestroyFn,
    execute_result_success: ExecuteResultSuccessFn,
    execute_result_message: ExecuteResultMessageFn,
    execute_result_rows_changed: ExecuteResultRowsChangedFn,
    execute_result_last_insert_rowid: ExecuteResultLastInsertRowIdFn,
    execute_result_statements_executed: ExecuteResultStatementsExecutedFn,
    query_json_result_destroy: QueryJsonResultDestroyFn,
    query_json_result_json_data: QueryJsonResultJsonDataFn,
    query_json_result_row_count: QueryJsonResultRowCountFn,
    query_stream_destroy: QueryStreamDestroyFn,
    query_stream_chunk_count: QueryStreamChunkCountFn,
    query_stream_row_count: QueryStreamRowCountFn,
    query_stream_total_bytes: QueryStreamTotalBytesFn,
    query_stream_get_chunk: QueryStreamGetChunkFn,
    bytes_free: BytesFreeFn,
    database_tokenize_text: DatabaseTokenizeTextFn,
    tokenize_result_destroy: TokenizeResultDestroyFn,
    tokenize_result_normalized_text: TokenizeResultNormalizedTextFn,
    tokenize_result_fts_query: TokenizeResultFtsQueryFn,
    tokenize_result_token_count: TokenizeResultTokenCountFn,
    tokenize_result_get_token: TokenizeResultGetTokenFn,
    database_upsert_custom_word: DatabaseUpsertCustomWordFn,
    database_remove_custom_word: DatabaseRemoveCustomWordFn,
    database_list_custom_words: DatabaseListCustomWordsFn,
    custom_word_list_destroy: CustomWordListDestroyFn,
    custom_word_list_len: CustomWordListLenFn,
    custom_word_list_get_word: CustomWordListGetWordFn,
    custom_word_list_get_weight: CustomWordListGetWeightFn,
    database_ensure_fts_index: DatabaseEnsureFtsIndexFn,
    database_rebuild_fts_index: DatabaseRebuildFtsIndexFn,
    database_upsert_fts_document: DatabaseUpsertFtsDocumentFn,
    database_delete_fts_document: DatabaseDeleteFtsDocumentFn,
    database_search_fts: DatabaseSearchFtsFn,
    search_result_destroy: SearchResultDestroyFn,
    search_result_total: SearchResultTotalFn,
    search_result_len: SearchResultLenFn,
    search_result_source: SearchResultSourceFn,
    search_result_query_mode: SearchResultQueryModeFn,
    search_result_get_id: SearchResultGetIdFn,
    search_result_get_file_path: SearchResultGetFilePathFn,
    search_result_get_title: SearchResultGetTitleFn,
    search_result_get_title_highlight: SearchResultGetTitleHighlightFn,
    search_result_get_content_snippet: SearchResultGetContentSnippetFn,
    search_result_get_score: SearchResultGetScoreFn,
    search_result_get_rank: SearchResultGetRankFn,
    search_result_get_raw_score: SearchResultGetRawScoreFn,
}

/// The loaded library and function table stay immutable after initialization, while outer locks protect shared access.
/// 动态库句柄与函数表初始化后只读，跨线程共享由外层锁负责保护。
unsafe impl Send for LoadedSqliteApi {}
unsafe impl Sync for LoadedSqliteApi {}

impl LoadedSqliteApi {
    /// Load the SQLite dynamic library using host conventions, preferring an explicit environment variable and runtime directories.
    /// 按宿主约定加载 SQLite 动态库，优先查找显式环境变量和运行时目录。
    fn load(library_path: &Path) -> Result<Self, String> {
        if !library_path.exists() {
            return Err(format!(
                "SQLite dynamic library path does not exist: {}",
                library_path.display()
            ));
        }

        let library = unsafe { Library::new(library_path) }.map_err(|error| {
            format!(
                "failed to load {}: {}: {}",
                library_path.display(),
                error,
                error
            )
        })?;
        unsafe { Self::from_library(library_path.to_path_buf(), library) }
    }

    /// Copy required exported function pointers from the opened dynamic library while retaining the library handle.
    /// 从已打开的动态库中复制所需函数指针，并保留库句柄防止提前卸载。
    unsafe fn from_library(library_path: PathBuf, library: Library) -> Result<Self, String> {
        macro_rules! load_symbol {
            ($name:literal, $ty:ty) => {{
                unsafe {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|error| {
                            format!(
                                "failed to load symbol {} from {}: {}",
                                $name,
                                library_path.display(),
                                error
                            )
                        })?
                }
            }};
        }

        Ok(Self {
            runtime_create_default: load_symbol!(
                "vldb_sqlite_runtime_create_default",
                RuntimeCreateDefaultFn
            ),
            runtime_destroy: load_symbol!("vldb_sqlite_runtime_destroy", RuntimeDestroyFn),
            runtime_open_database: load_symbol!(
                "vldb_sqlite_runtime_open_database",
                RuntimeOpenDatabaseFn
            ),
            database_destroy: load_symbol!("vldb_sqlite_database_destroy", DatabaseDestroyFn),
            database_db_path: load_symbol!("vldb_sqlite_database_db_path", DatabaseDbPathFn),
            string_free: load_symbol!("vldb_sqlite_string_free", StringFreeFn),
            last_error_message: load_symbol!("vldb_sqlite_last_error_message", LastErrorMessageFn),
            clear_last_error: load_symbol!("vldb_sqlite_clear_last_error", ClearLastErrorFn),
            library_info_json: load_symbol!("vldb_sqlite_library_info_json", LibraryInfoJsonFn),
            database_execute_script: load_symbol!(
                "vldb_sqlite_database_execute_script",
                DatabaseExecuteScriptFn
            ),
            database_execute_batch: load_symbol!(
                "vldb_sqlite_database_execute_batch",
                DatabaseExecuteBatchFn
            ),
            database_query_json: load_symbol!(
                "vldb_sqlite_database_query_json",
                DatabaseQueryJsonFn
            ),
            database_query_stream: load_symbol!(
                "vldb_sqlite_database_query_stream",
                DatabaseQueryStreamFn
            ),
            execute_result_destroy: load_symbol!(
                "vldb_sqlite_execute_result_destroy",
                ExecuteResultDestroyFn
            ),
            execute_result_success: load_symbol!(
                "vldb_sqlite_execute_result_success",
                ExecuteResultSuccessFn
            ),
            execute_result_message: load_symbol!(
                "vldb_sqlite_execute_result_message",
                ExecuteResultMessageFn
            ),
            execute_result_rows_changed: load_symbol!(
                "vldb_sqlite_execute_result_rows_changed",
                ExecuteResultRowsChangedFn
            ),
            execute_result_last_insert_rowid: load_symbol!(
                "vldb_sqlite_execute_result_last_insert_rowid",
                ExecuteResultLastInsertRowIdFn
            ),
            execute_result_statements_executed: load_symbol!(
                "vldb_sqlite_execute_result_statements_executed",
                ExecuteResultStatementsExecutedFn
            ),
            query_json_result_destroy: load_symbol!(
                "vldb_sqlite_query_json_result_destroy",
                QueryJsonResultDestroyFn
            ),
            query_json_result_json_data: load_symbol!(
                "vldb_sqlite_query_json_result_json_data",
                QueryJsonResultJsonDataFn
            ),
            query_json_result_row_count: load_symbol!(
                "vldb_sqlite_query_json_result_row_count",
                QueryJsonResultRowCountFn
            ),
            query_stream_destroy: load_symbol!(
                "vldb_sqlite_query_stream_destroy",
                QueryStreamDestroyFn
            ),
            query_stream_chunk_count: load_symbol!(
                "vldb_sqlite_query_stream_chunk_count",
                QueryStreamChunkCountFn
            ),
            query_stream_row_count: load_symbol!(
                "vldb_sqlite_query_stream_row_count",
                QueryStreamRowCountFn
            ),
            query_stream_total_bytes: load_symbol!(
                "vldb_sqlite_query_stream_total_bytes",
                QueryStreamTotalBytesFn
            ),
            query_stream_get_chunk: load_symbol!(
                "vldb_sqlite_query_stream_get_chunk",
                QueryStreamGetChunkFn
            ),
            bytes_free: load_symbol!("vldb_sqlite_bytes_free", BytesFreeFn),
            database_tokenize_text: load_symbol!(
                "vldb_sqlite_database_tokenize_text",
                DatabaseTokenizeTextFn
            ),
            tokenize_result_destroy: load_symbol!(
                "vldb_sqlite_tokenize_result_destroy",
                TokenizeResultDestroyFn
            ),
            tokenize_result_normalized_text: load_symbol!(
                "vldb_sqlite_tokenize_result_normalized_text",
                TokenizeResultNormalizedTextFn
            ),
            tokenize_result_fts_query: load_symbol!(
                "vldb_sqlite_tokenize_result_fts_query",
                TokenizeResultFtsQueryFn
            ),
            tokenize_result_token_count: load_symbol!(
                "vldb_sqlite_tokenize_result_token_count",
                TokenizeResultTokenCountFn
            ),
            tokenize_result_get_token: load_symbol!(
                "vldb_sqlite_tokenize_result_get_token",
                TokenizeResultGetTokenFn
            ),
            database_upsert_custom_word: load_symbol!(
                "vldb_sqlite_database_upsert_custom_word",
                DatabaseUpsertCustomWordFn
            ),
            database_remove_custom_word: load_symbol!(
                "vldb_sqlite_database_remove_custom_word",
                DatabaseRemoveCustomWordFn
            ),
            database_list_custom_words: load_symbol!(
                "vldb_sqlite_database_list_custom_words",
                DatabaseListCustomWordsFn
            ),
            custom_word_list_destroy: load_symbol!(
                "vldb_sqlite_custom_word_list_destroy",
                CustomWordListDestroyFn
            ),
            custom_word_list_len: load_symbol!(
                "vldb_sqlite_custom_word_list_len",
                CustomWordListLenFn
            ),
            custom_word_list_get_word: load_symbol!(
                "vldb_sqlite_custom_word_list_get_word",
                CustomWordListGetWordFn
            ),
            custom_word_list_get_weight: load_symbol!(
                "vldb_sqlite_custom_word_list_get_weight",
                CustomWordListGetWeightFn
            ),
            database_ensure_fts_index: load_symbol!(
                "vldb_sqlite_database_ensure_fts_index",
                DatabaseEnsureFtsIndexFn
            ),
            database_rebuild_fts_index: load_symbol!(
                "vldb_sqlite_database_rebuild_fts_index",
                DatabaseRebuildFtsIndexFn
            ),
            database_upsert_fts_document: load_symbol!(
                "vldb_sqlite_database_upsert_fts_document",
                DatabaseUpsertFtsDocumentFn
            ),
            database_delete_fts_document: load_symbol!(
                "vldb_sqlite_database_delete_fts_document",
                DatabaseDeleteFtsDocumentFn
            ),
            database_search_fts: load_symbol!(
                "vldb_sqlite_database_search_fts",
                DatabaseSearchFtsFn
            ),
            search_result_destroy: load_symbol!(
                "vldb_sqlite_search_result_destroy",
                SearchResultDestroyFn
            ),
            search_result_total: load_symbol!(
                "vldb_sqlite_search_result_total",
                SearchResultTotalFn
            ),
            search_result_len: load_symbol!("vldb_sqlite_search_result_len", SearchResultLenFn),
            search_result_source: load_symbol!(
                "vldb_sqlite_search_result_source",
                SearchResultSourceFn
            ),
            search_result_query_mode: load_symbol!(
                "vldb_sqlite_search_result_query_mode",
                SearchResultQueryModeFn
            ),
            search_result_get_id: load_symbol!(
                "vldb_sqlite_search_result_get_id",
                SearchResultGetIdFn
            ),
            search_result_get_file_path: load_symbol!(
                "vldb_sqlite_search_result_get_file_path",
                SearchResultGetFilePathFn
            ),
            search_result_get_title: load_symbol!(
                "vldb_sqlite_search_result_get_title",
                SearchResultGetTitleFn
            ),
            search_result_get_title_highlight: load_symbol!(
                "vldb_sqlite_search_result_get_title_highlight",
                SearchResultGetTitleHighlightFn
            ),
            search_result_get_content_snippet: load_symbol!(
                "vldb_sqlite_search_result_get_content_snippet",
                SearchResultGetContentSnippetFn
            ),
            search_result_get_score: load_symbol!(
                "vldb_sqlite_search_result_get_score",
                SearchResultGetScoreFn
            ),
            search_result_get_rank: load_symbol!(
                "vldb_sqlite_search_result_get_rank",
                SearchResultGetRankFn
            ),
            search_result_get_raw_score: load_symbol!(
                "vldb_sqlite_search_result_get_raw_score",
                SearchResultGetRawScoreFn
            ),
            _library: library,
            library_path,
        })
    }

    /// Read the latest FFI error and convert it into a stable Rust string.
    /// 读取最近一次 FFI 调用错误并转换成稳定 Rust 字符串。
    fn take_last_error_message(&self) -> String {
        unsafe {
            let ptr = (self.last_error_message)();
            let text = if ptr.is_null() {
                "unknown SQLite host error".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().to_string()
            };
            (self.clear_last_error)();
            text
        }
    }

    /// Convert a dynamic-library allocated string into a Rust `String` and free the original allocation.
    /// 释放动态库分配的字符串并转换成 Rust `String`。
    fn take_owned_string(&self, ptr: *mut c_char) -> Result<String, String> {
        if ptr.is_null() {
            return Err(self.take_last_error_message());
        }

        unsafe {
            let text = CStr::from_ptr(ptr).to_string_lossy().to_string();
            (self.string_free)(ptr);
            Ok(text)
        }
    }

    /// Convert a dynamic-library allocated optional string into Rust `Option<String>`.
    /// 将动态库分配的可选字符串转换成 Rust `Option<String>`。
    fn take_optional_string(&self, ptr: *mut c_char) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        unsafe {
            let text = CStr::from_ptr(ptr).to_string_lossy().to_string();
            (self.string_free)(ptr);
            Some(text)
        }
    }

    /// Invoke a zero-argument JSON FFI entrypoint and parse the response into `serde_json::Value`.
    /// 调用无参 JSON FFI 接口并解析成 `serde_json::Value`。
    fn call_json_noarg(
        &self,
        function: LibraryInfoJsonFn,
        operation: &str,
    ) -> Result<Value, String> {
        unsafe {
            let response_ptr = function();
            let response_text = self.take_owned_string(response_ptr)?;
            serde_json::from_str(&response_text)
                .map_err(|error| format!("{} returned invalid JSON: {}", operation, error))
        }
    }

    /// Copy a QueryStream byte buffer into a host-owned `Vec<u8>` and free the underlying allocation.
    /// 把 QueryStream 返回的字节缓冲区复制成宿主拥有的 `Vec<u8>`，并回收底层分配。
    fn take_chunk_bytes(&self, buffer: VldbSqliteByteBuffer) -> Result<Vec<u8>, String> {
        if buffer.data.is_null() {
            if buffer.len == 0 {
                return Ok(Vec::new());
            }
            return Err(self.take_last_error_message());
        }

        let len =
            usize::try_from(buffer.len).map_err(|_| "chunk length exceeds usize".to_string())?;
        unsafe {
            let bytes = std::slice::from_raw_parts(buffer.data, len).to_vec();
            (self.bytes_free)(buffer);
            Ok(bytes)
        }
    }
}

/// SQLite handle set for a single skill, with lifetime managed centrally by the host.
/// 单个 skill 的 SQLite 句柄集合，由宿主统一管理生命周期。
struct SkillHandleState {
    runtime: *mut VldbSqliteRuntimeHandle,
    database: *mut VldbSqliteDatabaseHandle,
    query_streams: HashMap<u64, *mut VldbSqliteQueryStreamHandle>,
    next_stream_id: u64,
}

/// FFI handles are accessed only behind a host-side mutex, with all cross-thread sharing managed by the host.
/// FFI 句柄仅通过宿主互斥量串行访问，跨线程共享由宿主统一控制。
unsafe impl Send for SkillHandleState {}

/// Database context bound to one SQLite-enabled skill.
/// 启用 SQLite 的 skill 所绑定的数据库上下文。
pub struct SqliteSkillBinding {
    api: Arc<LoadedSqliteApi>,
    skill_name: String,
    skill_dir_name: String,
    database_path: String,
    config: SkillSqliteMeta,
    handles: Mutex<SkillHandleState>,
}

impl SqliteSkillBinding {
    /// Return the stable SQLite status payload for the current skill; the response shape stays stable whether enabled or disabled.
    /// 返回当前 skill 的稳定 SQLite 状态信息；无论启用与否，结构都保持稳定。
    pub fn status_json(&self) -> Value {
        let library_info = self
            .api
            .call_json_noarg(self.api.library_info_json, "library_info_json")
            .unwrap_or_else(|error| {
                json!({
                    "name": "vldb-sqlite",
                    "version": "unknown",
                    "ffi_stage": "unknown",
                    "capabilities": [],
                    "warning": error,
                })
            });
        json!({
            "enabled": true,
            "initialized": true,
            "skill_name": self.skill_name,
            "skill_dir_name": self.skill_dir_name,
            "database_path": self.database_path,
            "integration_mode": "dynamic_library",
            "library_path": self.api.library_path.to_string_lossy().to_string(),
            "library_name": library_info.get("name").cloned().unwrap_or(Value::String("vldb-sqlite".to_string())),
            "library_version": library_info.get("version").cloned().unwrap_or(Value::String("unknown".to_string())),
            "ffi_stage": library_info.get("ffi_stage").cloned().unwrap_or(Value::String("unknown".to_string())),
            "capabilities": library_info.get("capabilities").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
            "log_level": self.config.log_level.as_str(),
            "slow_log_enabled": self.config.slow_log_enabled,
            "slow_log_threshold_ms": self.config.slow_log_threshold_ms,
        })
    }

    /// Return basic information about the SQLite binding for the current skill.
    /// 返回当前 skill 所绑定 SQLite 的基础信息。
    pub fn info_json(&self) -> Value {
        let mut status = self.status_json();
        if let Some(status_object) = status.as_object_mut() {
            let library_info = self
                .api
                .call_json_noarg(self.api.library_info_json, "library_info_json")
                .unwrap_or_else(|error| {
                    json!({
                        "name": "vldb-sqlite",
                        "version": "unknown",
                        "ffi_stage": "unknown",
                        "capabilities": [],
                        "warning": error,
                    })
                });
            status_object.insert("library_info".to_string(), library_info);
        }
        status
    }

    /// Execute a script or single SQL statement through the non-JSON primary interface.
    /// 通过非 JSON 主接口执行脚本或单条 SQL。
    pub fn execute_script(&self, input: &Value) -> Result<Value, String> {
        let sql = require_string_field(input, "sql")?;
        let params = parse_single_sql_params(input)?;
        let owned_params = build_owned_ffi_values(&params)?;
        self.log_info("execute_script", None);
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let sql_cstr = to_cstring(sql, "sql")?;
        unsafe {
            let result_handle = (self.api.database_execute_script)(
                guard.database,
                sql_cstr.as_ptr(),
                if owned_params.values.is_empty() {
                    ptr::null()
                } else {
                    owned_params.as_ptr()
                },
                owned_params.len_u64(),
                ptr::null(),
            );
            if result_handle.is_null() {
                drop(guard);
                let error = self.api.take_last_error_message();
                self.log_warning("execute_script", &error);
                return Err(error);
            }

            let result = json!({
                "success": u8_to_bool((self.api.execute_result_success)(result_handle)),
                "message": self.api.take_optional_string((self.api.execute_result_message)(result_handle)).unwrap_or_default(),
                "rows_changed": (self.api.execute_result_rows_changed)(result_handle),
                "last_insert_rowid": (self.api.execute_result_last_insert_rowid)(result_handle),
                "statements_executed": (self.api.execute_result_statements_executed)(result_handle),
            });
            (self.api.execute_result_destroy)(result_handle);
            drop(guard);
            self.log_if_slow("execute_script", started_at, None);
            Ok(result)
        }
    }

    /// Execute batch SQL through the non-JSON primary interface.
    /// 通过非 JSON 主接口批量执行 SQL。
    pub fn execute_batch(&self, input: &Value) -> Result<Value, String> {
        let sql = require_string_field(input, "sql")?;
        let rows = parse_batch_sql_params(input)?;
        let owned_rows = build_owned_ffi_value_matrix(&rows)?;
        self.log_info("execute_batch", None);
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let sql_cstr = to_cstring(sql, "sql")?;
        unsafe {
            let result_handle = (self.api.database_execute_batch)(
                guard.database,
                sql_cstr.as_ptr(),
                owned_rows.as_ptr(),
                owned_rows.len_u64(),
            );
            if result_handle.is_null() {
                drop(guard);
                let error = self.api.take_last_error_message();
                self.log_warning("execute_batch", &error);
                return Err(error);
            }

            let result = json!({
                "success": u8_to_bool((self.api.execute_result_success)(result_handle)),
                "message": self.api.take_optional_string((self.api.execute_result_message)(result_handle)).unwrap_or_default(),
                "rows_changed": (self.api.execute_result_rows_changed)(result_handle),
                "last_insert_rowid": (self.api.execute_result_last_insert_rowid)(result_handle),
                "statements_executed": (self.api.execute_result_statements_executed)(result_handle),
            });
            (self.api.execute_result_destroy)(result_handle);
            drop(guard);
            self.log_if_slow("execute_batch", started_at, None);
            Ok(result)
        }
    }

    /// Execute a JSON row-set query through the non-JSON primary interface.
    /// 通过非 JSON 主接口执行 JSON 行集查询。
    pub fn query_json(&self, input: &Value) -> Result<Value, String> {
        let sql = require_string_field(input, "sql")?;
        let params = parse_single_sql_params(input)?;
        let owned_params = build_owned_ffi_values(&params)?;
        self.log_info("query_json", None);
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let sql_cstr = to_cstring(sql, "sql")?;
        unsafe {
            let result_handle = (self.api.database_query_json)(
                guard.database,
                sql_cstr.as_ptr(),
                if owned_params.values.is_empty() {
                    ptr::null()
                } else {
                    owned_params.as_ptr()
                },
                owned_params.len_u64(),
                ptr::null(),
            );
            if result_handle.is_null() {
                drop(guard);
                let error = self.api.take_last_error_message();
                self.log_warning("query_json", &error);
                return Err(error);
            }

            let row_count = (self.api.query_json_result_row_count)(result_handle);
            let json_data = self
                .api
                .take_owned_string((self.api.query_json_result_json_data)(result_handle))?;
            let rows = serde_json::from_str::<Value>(&json_data)
                .map_err(|error| format!("query_json returned invalid json_data: {}", error))?;
            (self.api.query_json_result_destroy)(result_handle);
            drop(guard);
            self.log_if_slow(
                "query_json",
                started_at,
                Some(format!("rows={}", row_count)),
            );
            Ok(json!({
                "success": true,
                "row_count": row_count,
                "json_data": json_data,
                "rows": rows,
            }))
        }
    }

    /// Create a QueryStream handle through the non-JSON primary interface.
    /// 通过非 JSON 主接口创建 QueryStream 句柄。
    pub fn query_stream(&self, input: &Value) -> Result<Value, String> {
        let sql = require_string_field(input, "sql")?;
        let params = parse_single_sql_params(input)?;
        let owned_params = build_owned_ffi_values(&params)?;
        let chunk_bytes = input
            .get("chunk_bytes")
            .or_else(|| input.get("chunk_size"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.log_info("query_stream", None);
        let started_at = Instant::now();
        let mut guard = self.lock_handles()?;
        let sql_cstr = to_cstring(sql, "sql")?;
        unsafe {
            let result_handle = (self.api.database_query_stream)(
                guard.database,
                sql_cstr.as_ptr(),
                if owned_params.values.is_empty() {
                    ptr::null()
                } else {
                    owned_params.as_ptr()
                },
                owned_params.len_u64(),
                ptr::null(),
                chunk_bytes,
            );
            if result_handle.is_null() {
                drop(guard);
                let error = self.api.take_last_error_message();
                self.log_warning("query_stream", &error);
                return Err(error);
            }

            let stream_id = guard.next_stream_id;
            guard.next_stream_id = guard.next_stream_id.saturating_add(1).max(1);
            guard.query_streams.insert(stream_id, result_handle);
            drop(guard);
            self.log_if_slow(
                "query_stream",
                started_at,
                Some(format!("stream_id={} metrics_ready=false", stream_id)),
            );
            Ok(json!({
                "success": true,
                "stream_id": stream_id,
                "metrics_ready": false,
            }))
        }
    }

    /// Wait for final QueryStream metrics and return terminal statistics.
    /// 等待 QueryStream 最终统计信息就绪，并返回终态指标。
    pub fn query_stream_wait_metrics(&self, input: &Value) -> Result<Value, String> {
        let stream_id = input
            .get("stream_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "stream_id is required".to_string())?;
        self.log_info("query_stream_wait_metrics", None);
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let stream_handle = *guard
            .query_streams
            .get(&stream_id)
            .ok_or_else(|| format!("query stream handle not found: {}", stream_id))?;
        unsafe {
            let row_count = (self.api.query_stream_row_count)(stream_handle);
            let chunk_count = (self.api.query_stream_chunk_count)(stream_handle);
            let total_bytes = (self.api.query_stream_total_bytes)(stream_handle);
            drop(guard);
            self.log_if_slow(
                "query_stream_wait_metrics",
                started_at,
                Some(format!(
                    "stream_id={} chunks={} rows={} bytes={}",
                    stream_id, chunk_count, row_count, total_bytes
                )),
            );
            Ok(json!({
                "success": true,
                "stream_id": stream_id,
                "metrics_ready": true,
                "row_count": row_count,
                "chunk_count": chunk_count,
                "total_bytes": total_bytes,
            }))
        }
    }

    /// Read a single QueryStream chunk and return it as base64 text.
    /// 读取单个 QueryStream chunk，并以 base64 形式返回。
    pub fn query_stream_chunk(&self, input: &Value) -> Result<Value, String> {
        let stream_id = input
            .get("stream_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "stream_id is required".to_string())?;
        let index = input
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| "index is required".to_string())?;
        self.log_info("query_stream_chunk", None);
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let stream_handle = *guard
            .query_streams
            .get(&stream_id)
            .ok_or_else(|| format!("query stream handle not found: {}", stream_id))?;
        unsafe {
            let buffer = (self.api.query_stream_get_chunk)(stream_handle, index);
            let chunk = self.api.take_chunk_bytes(buffer)?;
            drop(guard);
            self.log_if_slow(
                "query_stream_chunk",
                started_at,
                Some(format!(
                    "stream_id={} index={} bytes={}",
                    stream_id,
                    index,
                    chunk.len()
                )),
            );
            Ok(json!({
                "success": true,
                "stream_id": stream_id,
                "index": index,
                "byte_count": u64::try_from(chunk.len()).unwrap_or(u64::MAX),
                "chunk_base64": BASE64_STANDARD.encode(chunk),
            }))
        }
    }

    /// Close a QueryStream handle and release the host-cached stream result.
    /// 关闭 QueryStream 句柄并释放宿主缓存的流结果。
    pub fn query_stream_close(&self, input: &Value) -> Result<Value, String> {
        let stream_id = input
            .get("stream_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "stream_id is required".to_string())?;
        self.log_info("query_stream_close", None);
        let started_at = Instant::now();
        let mut guard = self.lock_handles()?;
        let stream_handle = guard
            .query_streams
            .remove(&stream_id)
            .ok_or_else(|| format!("query stream handle not found: {}", stream_id))?;
        unsafe {
            (self.api.query_stream_destroy)(stream_handle);
            drop(guard);
            self.log_if_slow(
                "query_stream_close",
                started_at,
                Some(format!("stream_id={}", stream_id)),
            );
            Ok(json!({
                "success": true,
                "stream_id": stream_id,
                "message": format!("query_stream handle {} closed successfully", stream_id),
            }))
        }
    }

    /// Execute text tokenization and return a normalized result payload.
    /// 执行文本分词，并返回标准化结果。
    pub fn tokenize_text_json(&self, input: &Value) -> Result<Value, String> {
        let tokenizer_mode = parse_tokenizer_mode(
            input
                .get("tokenizer_mode")
                .or_else(|| input.get("mode"))
                .and_then(Value::as_str)
                .unwrap_or("none"),
        )?;
        let text = require_string_field(input, "text")?;
        let search_mode = input
            .get("search_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        self.log_info(
            "tokenize_text",
            Some(format!(
                "tokenizer_mode={} search_mode={}",
                tokenizer_mode_name(tokenizer_mode),
                search_mode
            )),
        );
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let text_cstr = to_cstring(text, "text")?;
        unsafe {
            let handle = (self.api.database_tokenize_text)(
                guard.database,
                tokenizer_mode,
                text_cstr.as_ptr(),
                bool_to_u8(search_mode),
            );
            if handle.is_null() {
                drop(guard);
                let error = self.api.take_last_error_message();
                self.log_warning("tokenize_text", &error);
                return Err(error);
            }

            let normalized_text =
                self.api
                    .take_owned_string((self.api.tokenize_result_normalized_text)(handle))?;
            let fts_query = self
                .api
                .take_owned_string((self.api.tokenize_result_fts_query)(handle))?;
            let token_count = (self.api.tokenize_result_token_count)(handle);
            let mut tokens = Vec::with_capacity(token_count as usize);
            for index in 0..token_count {
                if let Some(token) =
                    self.api
                        .take_optional_string((self.api.tokenize_result_get_token)(handle, index))
                {
                    tokens.push(Value::String(token));
                }
            }
            (self.api.tokenize_result_destroy)(handle);
            drop(guard);
            self.log_if_slow("tokenize_text", started_at, None);
            Ok(json!({
                "success": true,
                "tokenizer_mode": tokenizer_mode_name(tokenizer_mode),
                "normalized_text": normalized_text,
                "fts_query": fts_query,
                "tokens": tokens,
            }))
        }
    }

    /// Insert or update a custom dictionary word.
    /// 写入或更新自定义词。
    pub fn upsert_custom_word_json(&self, input: &Value) -> Result<Value, String> {
        let word = require_string_field(input, "word")?;
        let weight = input.get("weight").and_then(Value::as_u64).unwrap_or(1);
        self.log_info("upsert_custom_word", Some(format!("word={}", word)));
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let word_cstr = to_cstring(word, "word")?;
        let mut result = VldbSqliteDictionaryMutationResultPod {
            success: 0,
            affected_rows: 0,
        };
        let status = unsafe {
            (self.api.database_upsert_custom_word)(
                guard.database,
                word_cstr.as_ptr(),
                weight,
                &mut result,
            )
        };
        drop(guard);
        self.log_if_slow("upsert_custom_word", started_at, None);
        ensure_status(&self.api, status, "upsert_custom_word")?;
        Ok(json!({
            "success": u8_to_bool(result.success),
            "affected_rows": result.affected_rows,
            "word": word,
            "weight": weight,
        }))
    }

    /// Remove a custom dictionary word.
    /// 删除自定义词。
    pub fn remove_custom_word_json(&self, input: &Value) -> Result<Value, String> {
        let word = require_string_field(input, "word")?;
        self.log_info("remove_custom_word", Some(format!("word={}", word)));
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let word_cstr = to_cstring(word, "word")?;
        let mut result = VldbSqliteDictionaryMutationResultPod {
            success: 0,
            affected_rows: 0,
        };
        let status = unsafe {
            (self.api.database_remove_custom_word)(guard.database, word_cstr.as_ptr(), &mut result)
        };
        drop(guard);
        self.log_if_slow("remove_custom_word", started_at, None);
        ensure_status(&self.api, status, "remove_custom_word")?;
        Ok(json!({
            "success": u8_to_bool(result.success),
            "affected_rows": result.affected_rows,
            "word": word,
        }))
    }

    /// List enabled custom dictionary words from the current database.
    /// 列出当前数据库中启用的自定义词。
    pub fn list_custom_words_json(&self) -> Result<Value, String> {
        self.log_info("list_custom_words", None);
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        unsafe {
            let list_handle = (self.api.database_list_custom_words)(guard.database);
            if list_handle.is_null() {
                drop(guard);
                let error = self.api.take_last_error_message();
                self.log_warning("list_custom_words", &error);
                return Err(error);
            }

            let len = (self.api.custom_word_list_len)(list_handle);
            let mut words = Vec::with_capacity(len as usize);
            for index in 0..len {
                let word = self
                    .api
                    .take_optional_string((self.api.custom_word_list_get_word)(list_handle, index))
                    .unwrap_or_default();
                let weight = (self.api.custom_word_list_get_weight)(list_handle, index);
                words.push(json!({
                    "word": word,
                    "weight": weight,
                }));
            }
            (self.api.custom_word_list_destroy)(list_handle);
            drop(guard);
            self.log_if_slow(
                "list_custom_words",
                started_at,
                Some(format!("count={}", len)),
            );
            Ok(json!({
                "success": true,
                "total": len,
                "words": words,
            }))
        }
    }

    /// Ensure the specified FTS index exists.
    /// 确保指定 FTS 索引存在。
    pub fn ensure_fts_index_json(&self, input: &Value) -> Result<Value, String> {
        let index_name = require_string_field(input, "index_name")?;
        let tokenizer_mode = parse_tokenizer_mode(
            input
                .get("tokenizer_mode")
                .or_else(|| input.get("mode"))
                .and_then(Value::as_str)
                .unwrap_or("none"),
        )?;
        self.log_info(
            "ensure_fts_index",
            Some(format!("index_name={}", index_name)),
        );
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let index_cstr = to_cstring(index_name, "index_name")?;
        let mut result = VldbSqliteEnsureFtsIndexResultPod {
            success: 0,
            tokenizer_mode: tokenizer_mode as u32,
        };
        let status = unsafe {
            (self.api.database_ensure_fts_index)(
                guard.database,
                index_cstr.as_ptr(),
                tokenizer_mode,
                &mut result,
            )
        };
        drop(guard);
        self.log_if_slow("ensure_fts_index", started_at, None);
        ensure_status(&self.api, status, "ensure_fts_index")?;
        Ok(json!({
            "success": u8_to_bool(result.success),
            "index_name": index_name,
            "tokenizer_mode": tokenizer_mode_name_from_u32(result.tokenizer_mode),
        }))
    }

    /// Rebuild an FTS index using the current dictionary and tokenizer mode.
    /// 使用当前词典和分词模式重建 FTS 索引。
    pub fn rebuild_fts_index_json(&self, input: &Value) -> Result<Value, String> {
        let index_name = require_string_field(input, "index_name")?;
        let tokenizer_mode = parse_tokenizer_mode(
            input
                .get("tokenizer_mode")
                .or_else(|| input.get("mode"))
                .and_then(Value::as_str)
                .unwrap_or("none"),
        )?;
        self.log_info(
            "rebuild_fts_index",
            Some(format!("index_name={}", index_name)),
        );
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let index_cstr = to_cstring(index_name, "index_name")?;
        let mut result = VldbSqliteRebuildFtsIndexResultPod {
            success: 0,
            tokenizer_mode: tokenizer_mode as u32,
            reindexed_rows: 0,
        };
        let status = unsafe {
            (self.api.database_rebuild_fts_index)(
                guard.database,
                index_cstr.as_ptr(),
                tokenizer_mode,
                &mut result,
            )
        };
        drop(guard);
        self.log_if_slow("rebuild_fts_index", started_at, None);
        ensure_status(&self.api, status, "rebuild_fts_index")?;
        Ok(json!({
            "success": u8_to_bool(result.success),
            "index_name": index_name,
            "tokenizer_mode": tokenizer_mode_name_from_u32(result.tokenizer_mode),
            "reindexed_rows": result.reindexed_rows,
        }))
    }

    /// Insert or update a single FTS document.
    /// 写入或更新一条 FTS 文档。
    pub fn upsert_fts_document_json(&self, input: &Value) -> Result<Value, String> {
        let index_name = require_string_field(input, "index_name")?;
        let tokenizer_mode = parse_tokenizer_mode(
            input
                .get("tokenizer_mode")
                .or_else(|| input.get("mode"))
                .and_then(Value::as_str)
                .unwrap_or("none"),
        )?;
        let id = require_string_field(input, "id")?;
        let file_path = require_string_field(input, "file_path")?;
        let title = input.get("title").and_then(Value::as_str).unwrap_or("");
        let content = input.get("content").and_then(Value::as_str).unwrap_or("");
        self.log_info(
            "upsert_fts_document",
            Some(format!("index_name={} id={}", index_name, id)),
        );
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let index_cstr = to_cstring(index_name, "index_name")?;
        let id_cstr = to_cstring(id, "id")?;
        let file_path_cstr = to_cstring(file_path, "file_path")?;
        let title_cstr = to_cstring(title, "title")?;
        let content_cstr = to_cstring(content, "content")?;
        let mut result = VldbSqliteFtsMutationResultPod {
            success: 0,
            affected_rows: 0,
        };
        let status = unsafe {
            (self.api.database_upsert_fts_document)(
                guard.database,
                index_cstr.as_ptr(),
                tokenizer_mode,
                id_cstr.as_ptr(),
                file_path_cstr.as_ptr(),
                title_cstr.as_ptr(),
                content_cstr.as_ptr(),
                &mut result,
            )
        };
        drop(guard);
        self.log_if_slow("upsert_fts_document", started_at, None);
        ensure_status(&self.api, status, "upsert_fts_document")?;
        Ok(json!({
            "success": u8_to_bool(result.success),
            "affected_rows": result.affected_rows,
            "index_name": index_name,
            "id": id,
        }))
    }

    /// Delete a single FTS document.
    /// 删除一条 FTS 文档。
    pub fn delete_fts_document_json(&self, input: &Value) -> Result<Value, String> {
        let index_name = require_string_field(input, "index_name")?;
        let id = require_string_field(input, "id")?;
        self.log_info(
            "delete_fts_document",
            Some(format!("index_name={} id={}", index_name, id)),
        );
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let index_cstr = to_cstring(index_name, "index_name")?;
        let id_cstr = to_cstring(id, "id")?;
        let mut result = VldbSqliteFtsMutationResultPod {
            success: 0,
            affected_rows: 0,
        };
        let status = unsafe {
            (self.api.database_delete_fts_document)(
                guard.database,
                index_cstr.as_ptr(),
                id_cstr.as_ptr(),
                &mut result,
            )
        };
        drop(guard);
        self.log_if_slow("delete_fts_document", started_at, None);
        ensure_status(&self.api, status, "delete_fts_document")?;
        Ok(json!({
            "success": u8_to_bool(result.success),
            "affected_rows": result.affected_rows,
            "index_name": index_name,
            "id": id,
        }))
    }

    /// Execute FTS search and return a rich result payload.
    /// 执行 FTS 检索并返回富结果结构。
    pub fn search_fts_json(&self, input: &Value) -> Result<Value, String> {
        let index_name = require_string_field(input, "index_name")?;
        let tokenizer_mode = parse_tokenizer_mode(
            input
                .get("tokenizer_mode")
                .or_else(|| input.get("mode"))
                .and_then(Value::as_str)
                .unwrap_or("none"),
        )?;
        let query = require_string_field(input, "query")?;
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(10) as u32;
        let offset = input.get("offset").and_then(Value::as_u64).unwrap_or(0) as u32;
        self.log_info(
            "search_fts",
            Some(format!(
                "index_name={} tokenizer_mode={} limit={} offset={}",
                index_name,
                tokenizer_mode_name(tokenizer_mode),
                limit,
                offset
            )),
        );
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let index_cstr = to_cstring(index_name, "index_name")?;
        let query_cstr = to_cstring(query, "query")?;
        unsafe {
            let result_handle = (self.api.database_search_fts)(
                guard.database,
                index_cstr.as_ptr(),
                tokenizer_mode,
                query_cstr.as_ptr(),
                limit,
                offset,
            );
            if result_handle.is_null() {
                drop(guard);
                let error = self.api.take_last_error_message();
                self.log_warning("search_fts", &error);
                return Err(error);
            }

            let total = (self.api.search_result_total)(result_handle);
            let len = (self.api.search_result_len)(result_handle);
            let source = self
                .api
                .take_optional_string((self.api.search_result_source)(result_handle))
                .unwrap_or_else(|| "sqlite_fts".to_string());
            let query_mode = self
                .api
                .take_optional_string((self.api.search_result_query_mode)(result_handle))
                .unwrap_or_else(|| "fts".to_string());
            let mut hits = Vec::with_capacity(len as usize);
            for index in 0..len {
                hits.push(json!({
                    "id": self.api.take_optional_string((self.api.search_result_get_id)(result_handle, index)).unwrap_or_default(),
                    "file_path": self.api.take_optional_string((self.api.search_result_get_file_path)(result_handle, index)).unwrap_or_default(),
                    "title": self.api.take_optional_string((self.api.search_result_get_title)(result_handle, index)).unwrap_or_default(),
                    "title_highlight": self.api.take_optional_string((self.api.search_result_get_title_highlight)(result_handle, index)).unwrap_or_default(),
                    "content_snippet": self.api.take_optional_string((self.api.search_result_get_content_snippet)(result_handle, index)).unwrap_or_default(),
                    "score": (self.api.search_result_get_score)(result_handle, index),
                    "rank": (self.api.search_result_get_rank)(result_handle, index),
                    "raw_score": (self.api.search_result_get_raw_score)(result_handle, index),
                }));
            }
            (self.api.search_result_destroy)(result_handle);
            drop(guard);
            self.log_if_slow("search_fts", started_at, Some(format!("hits={}", len)));
            Ok(json!({
                "success": true,
                "index_name": index_name,
                "tokenizer_mode": tokenizer_mode_name(tokenizer_mode),
                "source": source,
                "query_mode": query_mode,
                "total": total,
                "hits": hits,
            }))
        }
    }

    /// Emit informational logs according to the configured skill policy.
    /// 按配置输出普通信息级日志。
    fn log_info(&self, operation: &str, extra: Option<String>) {
        if self.config.log_level == SkillSqliteLogLevel::Info {
            match extra {
                Some(extra) => log_info(format!(
                    "[Sqlite:info] skill={} db={} op={} {}",
                    self.skill_name, self.skill_dir_name, operation, extra
                )),
                None => log_info(format!(
                    "[Sqlite:info] skill={} db={} op={}",
                    self.skill_name, self.skill_dir_name, operation
                )),
            }
        }
    }

    /// Emit slow-operation warnings according to the slow-log configuration.
    /// 按慢日志配置输出慢操作告警。
    fn log_if_slow(&self, operation: &str, started_at: Instant, extra: Option<String>) {
        if !self.config.slow_log_enabled {
            return;
        }
        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        if elapsed_ms < self.config.slow_log_threshold_ms {
            return;
        }
        match extra {
            Some(extra) => log_info(format!(
                "[Sqlite:slow] skill={} db={} op={} elapsed_ms={} {}",
                self.skill_name, self.skill_dir_name, operation, elapsed_ms, extra
            )),
            None => log_info(format!(
                "[Sqlite:slow] skill={} db={} op={} elapsed_ms={}",
                self.skill_name, self.skill_dir_name, operation, elapsed_ms
            )),
        }
    }

    /// Emit warning-level logs according to configuration, usually for FFI call failures.
    /// 按配置输出告警级日志，通常用于 FFI 调用失败。
    fn log_warning(&self, operation: &str, message: &str) {
        if matches!(
            self.config.log_level,
            SkillSqliteLogLevel::Info | SkillSqliteLogLevel::Warning
        ) {
            log_warn(format!(
                "[Sqlite:warn] skill={} db={} op={} message={}",
                self.skill_name, self.skill_dir_name, operation, message
            ));
        }
    }

    /// Acquire the handle lock so SQLite FFI calls for the same skill execute serially.
    /// 获取句柄锁，确保同一个 skill 的 SQLite FFI 调用按顺序串行执行。
    fn lock_handles(&self) -> Result<std::sync::MutexGuard<'_, SkillHandleState>, String> {
        self.handles
            .lock()
            .map_err(|_| "failed to acquire SQLite handle lock".to_string())
    }
}

impl Drop for SqliteSkillBinding {
    /// Release the database handle and runtime together when the skill binding is dropped.
    /// 在 skill 生命周期结束时统一释放数据库句柄与 runtime。
    fn drop(&mut self) {
        if let Ok(mut guard) = self.handles.lock() {
            unsafe {
                for (_, stream_handle) in guard.query_streams.drain() {
                    if !stream_handle.is_null() {
                        (self.api.query_stream_destroy)(stream_handle);
                    }
                }
                if !guard.database.is_null() {
                    (self.api.database_destroy)(guard.database);
                    guard.database = ptr::null_mut();
                }
                if !guard.runtime.is_null() {
                    (self.api.runtime_destroy)(guard.runtime);
                    guard.runtime = ptr::null_mut();
                }
            }
        }
    }
}

/// Maintain SQLite bindings per skill, auto-creating and reusing them for enabled skills.
/// 按 skill 维度维护 SQLite 绑定，负责启用后的自动创建与长期复用。
pub struct SqliteSkillHost {
    api: Arc<LoadedSqliteApi>,
    skills: Mutex<HashMap<String, Arc<SqliteSkillBinding>>>,
    host_options: LuaRuntimeHostOptions,
}

impl SqliteSkillHost {
    /// Create the host-side SQLite skill manager and load the dynamic library immediately.
    /// 创建宿主级 SQLite 技能管理器，并立即加载动态库。
    pub fn new(host_options: LuaRuntimeHostOptions) -> Result<Self, String> {
        let library_path = host_options
            .sqlite_library_path
            .clone()
            .ok_or_else(|| "SQLite host requires host_options.sqlite_library_path".to_string())?;
        Ok(Self {
            api: Arc::new(LoadedSqliteApi::load(&library_path)?),
            skills: Mutex::new(HashMap::new()),
            host_options,
        })
    }

    /// Register a fixed database binding for an SQLite-enabled skill; each skill is created only once.
    /// 为启用 SQLite 的 skill 注册固定数据库绑定；同一个 skill 只会创建一次。
    pub fn register_skill(
        &self,
        skill_name: &str,
        skill_dir: &Path,
        config: SkillSqliteMeta,
    ) -> Result<Arc<SqliteSkillBinding>, String> {
        let mut guard = self
            .skills
            .lock()
            .map_err(|_| "failed to acquire SQLite skill registry lock".to_string())?;
        if let Some(existing) = guard.get(skill_name) {
            return Ok(existing.clone());
        }

        let skill_dir_name = skill_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "invalid skill directory name for {}: {}",
                    skill_name,
                    skill_dir.display()
                )
            })?
            .to_string();
        let skills_root = skill_dir.parent().ok_or_else(|| {
            format!(
                "invalid skill root for {}: {}",
                skill_name,
                skill_dir.display()
            )
        })?;
        let sidecar_root = skills_root
            .parent()
            .unwrap_or(skills_root)
            .join(self.host_options.database_dir_name.as_str());
        let db_dir = sidecar_root.join("sqlite").join(skill_name);
        std::fs::create_dir_all(&db_dir).map_err(|error| {
            format!(
                "failed to create SQLite directory {}: {}: {}",
                db_dir.display(),
                error,
                error
            )
        })?;
        let db_path = db_dir.join(format!("{}.sqlite3", skill_name));
        let database_path = db_path.to_string_lossy().to_string();
        let database_cstr = CString::new(database_path.clone())
            .map_err(|_| "database path contains interior NUL bytes".to_string())?;

        let runtime = unsafe { (self.api.runtime_create_default)() };
        if runtime.is_null() {
            return Err(self.api.take_last_error_message());
        }

        let database = unsafe { (self.api.runtime_open_database)(runtime, database_cstr.as_ptr()) };
        if database.is_null() {
            unsafe {
                (self.api.runtime_destroy)(runtime);
            }
            return Err(self.api.take_last_error_message());
        }

        let resolved_path = unsafe {
            self.api
                .take_owned_string((self.api.database_db_path)(database))
        }
        .unwrap_or(database_path.clone());

        let binding = Arc::new(SqliteSkillBinding {
            api: self.api.clone(),
            skill_name: skill_name.to_string(),
            skill_dir_name,
            database_path: resolved_path,
            config,
            handles: Mutex::new(SkillHandleState {
                runtime,
                database,
                query_streams: HashMap::new(),
                next_stream_id: 1,
            }),
        });
        guard.insert(skill_name.to_string(), binding.clone());
        Ok(binding)
    }

    /// Fetch a registered binding by skill name so Lua injection and cross-skill calls can restore context.
    /// 按 skill 名称获取已注册绑定，供 Lua 注入与跨 skill 调用恢复上下文使用。
    pub fn binding_for_skill(
        &self,
        skill_name: &str,
    ) -> Result<Option<Arc<SqliteSkillBinding>>, String> {
        let skills = self
            .skills
            .lock()
            .map_err(|_| "SQLite skill binding registry lock poisoned".to_string())?;
        Ok(skills.get(skill_name).cloned())
    }
}

/// Build a stable status object for skills without SQLite enabled so Lua can check before calling.
/// 为未启用 SQLite 的 skill 生成稳定状态对象，便于 Lua 侧先判断再调用。
pub fn disabled_skill_status_json(skill_name: Option<&str>) -> Value {
    json!({
        "enabled": false,
        "initialized": false,
        "skill_name": skill_name.unwrap_or(""),
        "integration_mode": "dynamic_library",
        "reason": "current skill has not enabled sqlite"
    })
}

/// Parse a tokenizer-mode text label into the FFI enum.
/// 将文本分词模式字符串解析为 FFI 枚举。
fn parse_tokenizer_mode(text: &str) -> Result<VldbSqliteFfiTokenizerMode, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "" | "none" => Ok(VldbSqliteFfiTokenizerMode::None),
        "jieba" => Ok(VldbSqliteFfiTokenizerMode::Jieba),
        other => Err(format!("unsupported sqlite tokenizer mode: {}", other)),
    }
}

/// Convert the FFI tokenizer mode into a stable string label.
/// 将 FFI 分词模式转换成稳定字符串。
fn tokenizer_mode_name(mode: VldbSqliteFfiTokenizerMode) -> &'static str {
    match mode {
        VldbSqliteFfiTokenizerMode::None => "none",
        VldbSqliteFfiTokenizerMode::Jieba => "jieba",
    }
}

/// Convert the tokenizer-mode integer returned by FFI into a stable string label.
/// 将 FFI 返回的分词模式数值转换成稳定字符串。
fn tokenizer_mode_name_from_u32(mode: u32) -> &'static str {
    match mode {
        1 => "jieba",
        _ => "none",
    }
}

/// Encode a boolean value as the `u8` representation used by the FFI.
/// 将布尔值编码为 FFI 所使用的 `u8`。
fn bool_to_u8(value: bool) -> u8 {
    if value { 1 } else { 0 }
}

/// Convert an FFI `u8` boolean into a Rust boolean.
/// 将 FFI `u8` 布尔值转换为 Rust 布尔值。
fn u8_to_bool(value: u8) -> bool {
    value != 0
}

/// Host-side SQLite parameter representation used as a stable bridge between Lua/JSON and the FFI ABI.
/// 宿主内部使用的 SQLite 参数值表示，负责在 Lua/JSON 与 FFI ABI 之间做稳定过渡。
enum HostSqliteParamValue {
    Null,
    Int64(i64),
    Float64(f64),
    String(String),
    Bytes(Vec<u8>),
    Bool(bool),
}

/// One owned FFI parameter array that keeps strings and byte buffers alive for the entire call.
/// 一组已拥有生命周期的 FFI 参数数组，确保字符串和字节缓冲在调用期间保持有效。
struct OwnedSqliteFfiValues {
    values: Vec<VldbSqliteFfiValue>,
    _strings: Vec<CString>,
    _bytes: Vec<Vec<u8>>,
}

impl OwnedSqliteFfiValues {
    /// Return the pointer to the first FFI parameter value.
    /// 返回 FFI 参数数组首指针。
    fn as_ptr(&self) -> *const VldbSqliteFfiValue {
        self.values.as_ptr()
    }

    /// Return the length of the FFI parameter array.
    /// 返回 FFI 参数数组长度。
    fn len_u64(&self) -> u64 {
        u64::try_from(self.values.len()).unwrap_or(u64::MAX)
    }
}

/// Owned two-dimensional parameter matrix used by batch SQL execution.
/// 批量 SQL 所使用的已拥有生命周期的二维参数矩阵。
struct OwnedSqliteFfiValueMatrix {
    _rows: Vec<OwnedSqliteFfiValues>,
    slices: Vec<VldbSqliteFfiValueSlice>,
}

impl OwnedSqliteFfiValueMatrix {
    /// Return the pointer to the first batch-parameter slice.
    /// 返回批量参数切片首指针。
    fn as_ptr(&self) -> *const VldbSqliteFfiValueSlice {
        self.slices.as_ptr()
    }

    /// Return the number of batch-parameter slices.
    /// 返回批量参数切片数量。
    fn len_u64(&self) -> u64 {
        u64::try_from(self.slices.len()).unwrap_or(u64::MAX)
    }
}

/// Convert host parameter values into an owned FFI parameter array.
/// 把宿主参数值数组转换成拥有生命周期的 FFI 参数数组。
fn build_owned_ffi_values(values: &[HostSqliteParamValue]) -> Result<OwnedSqliteFfiValues, String> {
    let mut ffi_values = Vec::with_capacity(values.len());
    let mut strings = Vec::new();
    let mut bytes = Vec::new();

    for value in values {
        match value {
            HostSqliteParamValue::Null => ffi_values.push(VldbSqliteFfiValue {
                kind: VldbSqliteFfiValueKind::Null,
                int64_value: 0,
                float64_value: 0.0,
                string_value: ptr::null(),
                bytes_value: VldbSqliteByteView::default(),
                bool_value: 0,
            }),
            HostSqliteParamValue::Int64(number) => ffi_values.push(VldbSqliteFfiValue {
                kind: VldbSqliteFfiValueKind::Int64,
                int64_value: *number,
                float64_value: 0.0,
                string_value: ptr::null(),
                bytes_value: VldbSqliteByteView::default(),
                bool_value: 0,
            }),
            HostSqliteParamValue::Float64(number) => ffi_values.push(VldbSqliteFfiValue {
                kind: VldbSqliteFfiValueKind::Float64,
                int64_value: 0,
                float64_value: *number,
                string_value: ptr::null(),
                bytes_value: VldbSqliteByteView::default(),
                bool_value: 0,
            }),
            HostSqliteParamValue::String(text) => {
                let c_text = to_cstring(text, "params[*].string")?;
                let ptr = c_text.as_ptr();
                strings.push(c_text);
                ffi_values.push(VldbSqliteFfiValue {
                    kind: VldbSqliteFfiValueKind::String,
                    int64_value: 0,
                    float64_value: 0.0,
                    string_value: ptr,
                    bytes_value: VldbSqliteByteView::default(),
                    bool_value: 0,
                });
            }
            HostSqliteParamValue::Bytes(blob) => {
                let owned = blob.clone();
                let view = if owned.is_empty() {
                    VldbSqliteByteView::default()
                } else {
                    VldbSqliteByteView {
                        data: owned.as_ptr(),
                        len: u64::try_from(owned.len()).unwrap_or(u64::MAX),
                    }
                };
                bytes.push(owned);
                ffi_values.push(VldbSqliteFfiValue {
                    kind: VldbSqliteFfiValueKind::Bytes,
                    int64_value: 0,
                    float64_value: 0.0,
                    string_value: ptr::null(),
                    bytes_value: view,
                    bool_value: 0,
                });
            }
            HostSqliteParamValue::Bool(flag) => ffi_values.push(VldbSqliteFfiValue {
                kind: VldbSqliteFfiValueKind::Bool,
                int64_value: 0,
                float64_value: 0.0,
                string_value: ptr::null(),
                bytes_value: VldbSqliteByteView::default(),
                bool_value: bool_to_u8(*flag),
            }),
        }
    }

    Ok(OwnedSqliteFfiValues {
        values: ffi_values,
        _strings: strings,
        _bytes: bytes,
    })
}

/// Convert a batch parameter matrix into owned FFI batch-parameter slices.
/// 把批量参数矩阵转换成拥有生命周期的 FFI 批量参数切片。
fn build_owned_ffi_value_matrix(
    rows: &[Vec<HostSqliteParamValue>],
) -> Result<OwnedSqliteFfiValueMatrix, String> {
    let owned_rows = rows
        .iter()
        .map(|row| build_owned_ffi_values(row))
        .collect::<Result<Vec<_>, _>>()?;
    let slices = owned_rows
        .iter()
        .map(|row| VldbSqliteFfiValueSlice {
            values: row.as_ptr(),
            len: row.len_u64(),
        })
        .collect::<Vec<_>>();
    Ok(OwnedSqliteFfiValueMatrix {
        _rows: owned_rows,
        slices,
    })
}

/// Convert a JSON/Lua scalar parameter into the host-side SQLite parameter representation.
/// 把 JSON/ Lua 标量参数转换为宿主内部 SQLite 参数值。
fn parse_scalar_sqlite_param(
    value: &Value,
    field_name: &str,
) -> Result<HostSqliteParamValue, String> {
    match value {
        Value::Null => Ok(HostSqliteParamValue::Null),
        Value::Bool(flag) => Ok(HostSqliteParamValue::Bool(*flag)),
        Value::Number(number) => {
            if let Some(int_value) = number.as_i64() {
                Ok(HostSqliteParamValue::Int64(int_value))
            } else if let Some(unsigned) = number.as_u64() {
                let converted = i64::try_from(unsigned).map_err(|_| {
                    format!(
                        "{} contains an unsigned integer larger than i64",
                        field_name
                    )
                })?;
                Ok(HostSqliteParamValue::Int64(converted))
            } else if let Some(float_value) = number.as_f64() {
                Ok(HostSqliteParamValue::Float64(float_value))
            } else {
                Err(format!(
                    "{} contains an unsupported numeric value",
                    field_name
                ))
            }
        }
        Value::String(text) => Ok(HostSqliteParamValue::String(text.clone())),
        _ => Err(format!("{} must contain only scalar values", field_name)),
    }
}

/// Convert a typed parameter object into the host-side SQLite parameter representation.
/// 把 typed 参数对象转换为宿主内部 SQLite 参数值。
fn parse_typed_sqlite_param(
    object: &serde_json::Map<String, Value>,
    field_name: &str,
) -> Result<HostSqliteParamValue, String> {
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{}.kind is required for typed parameters", field_name))?;
    match kind.trim().to_ascii_lowercase().as_str() {
        "null" => Ok(HostSqliteParamValue::Null),
        "bool" => object
            .get("value")
            .and_then(Value::as_bool)
            .map(HostSqliteParamValue::Bool)
            .ok_or_else(|| format!("{}.value must be a bool", field_name)),
        "int64" => object
            .get("value")
            .and_then(Value::as_i64)
            .map(HostSqliteParamValue::Int64)
            .ok_or_else(|| format!("{}.value must be an int64", field_name)),
        "float64" => object
            .get("value")
            .and_then(Value::as_f64)
            .map(HostSqliteParamValue::Float64)
            .ok_or_else(|| format!("{}.value must be a float64", field_name)),
        "string" => object
            .get("value")
            .and_then(Value::as_str)
            .map(|value| HostSqliteParamValue::String(value.to_string()))
            .ok_or_else(|| format!("{}.value must be a string", field_name)),
        "bytes" => {
            if let Some(base64_value) = object.get("base64").and_then(Value::as_str) {
                let decoded = BASE64_STANDARD
                    .decode(base64_value)
                    .map_err(|error| format!("{}.base64 is invalid: {}", field_name, error))?;
                return Ok(HostSqliteParamValue::Bytes(decoded));
            }
            let array = object
                .get("value")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    format!(
                        "{}.value must be a byte array or provide base64",
                        field_name
                    )
                })?;
            let mut bytes = Vec::with_capacity(array.len());
            for (index, item) in array.iter().enumerate() {
                let byte = item.as_u64().ok_or_else(|| {
                    format!(
                        "{}.value[{}] must be an unsigned integer",
                        field_name, index
                    )
                })?;
                let converted = u8::try_from(byte)
                    .map_err(|_| format!("{}.value[{}] exceeds u8", field_name, index))?;
                bytes.push(converted);
            }
            Ok(HostSqliteParamValue::Bytes(bytes))
        }
        other => Err(format!("{}.kind={} is unsupported", field_name, other)),
    }
}

/// Normalize a JSON parameter value into the host-side SQLite parameter representation.
/// 把 JSON 参数值统一转换为宿主内部 SQLite 参数值。
fn parse_sqlite_param(value: &Value, field_name: &str) -> Result<HostSqliteParamValue, String> {
    match value {
        Value::Object(object) if object.contains_key("kind") => {
            parse_typed_sqlite_param(object, field_name)
        }
        other => parse_scalar_sqlite_param(other, field_name),
    }
}

/// Parse legacy `params_json` text, allowing scalar arrays only.
/// 解析 legacy `params_json` 字符串，只允许标量数组。
fn parse_legacy_params_json_text(params_json: &str) -> Result<Vec<HostSqliteParamValue>, String> {
    if params_json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Value = serde_json::from_str(params_json).map_err(|error| {
        format!(
            "params_json must be a JSON array of scalar values: {}",
            error
        )
    })?;
    let items = parsed
        .as_array()
        .ok_or_else(|| "params_json must be a JSON array of scalar values".to_string())?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| parse_scalar_sqlite_param(item, &format!("params_json[{}]", index)))
        .collect()
}

/// Parse the parameter list for a single SQL request from the unified input object.
/// 从统一输入对象中解析单条 SQL 的参数列表。
fn parse_single_sql_params(input: &Value) -> Result<Vec<HostSqliteParamValue>, String> {
    let params_json = input
        .get("params_json")
        .and_then(Value::as_str)
        .unwrap_or("");
    if let Some(params_value) = input.get("params") {
        if !params_json.trim().is_empty() {
            return Err("provide either params or params_json, but not both".to_string());
        }
        let params_array = params_value
            .as_array()
            .ok_or_else(|| "params must be an array".to_string())?;
        return params_array
            .iter()
            .enumerate()
            .map(|(index, item)| parse_sqlite_param(item, &format!("params[{}]", index)))
            .collect();
    }
    parse_legacy_params_json_text(params_json)
}

/// Parse the parameter matrix for batch SQL from the unified input object.
/// 从统一输入对象中解析批量 SQL 的参数矩阵。
fn parse_batch_sql_params(input: &Value) -> Result<Vec<Vec<HostSqliteParamValue>>, String> {
    let items = input
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "items must be an array of arrays".to_string())?;
    if items.is_empty() {
        return Err("items must not be empty".to_string());
    }
    items
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let row_items = row
                .as_array()
                .ok_or_else(|| format!("items[{}] must be an array", row_index))?;
            row_items
                .iter()
                .enumerate()
                .map(|(col_index, item)| {
                    parse_sqlite_param(item, &format!("items[{}][{}]", row_index, col_index))
                })
                .collect()
        })
        .collect()
}

/// Ensure that a required string field exists in the JSON request.
/// 确保 JSON 请求中存在指定字符串字段。
fn require_string_field<'a>(input: &'a Value, field_name: &str) -> Result<&'a str, String> {
    input
        .get(field_name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing or empty field `{}`", field_name))
}

/// Convert a Rust string into a C string while uniformly validating interior NUL bytes.
/// 将 Rust 字符串转换为 C 字符串，统一校验 NUL 字节。
fn to_cstring(text: &str, field_name: &str) -> Result<CString, String> {
    CString::new(text).map_err(|_| format!("field `{}` contains interior NUL bytes", field_name))
}

/// Check the FFI return status code and convert failures into a host-level error string.
/// 检查 FFI 返回状态码，并在失败时转换成宿主级错误文本。
fn ensure_status(api: &LoadedSqliteApi, status: i32, operation: &str) -> Result<(), String> {
    if status == VldbSqliteStatusCode::Success as i32 {
        return Ok(());
    }
    let error = api.take_last_error_message();
    Err(format!("{} failed: {}", operation, error))
}
