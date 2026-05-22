use super::host_result::validate_change_set_payload;
use super::lease::RuntimeSessionManager;
use super::runlua::runlua_cwd_guard;
use super::{
    LoadedSkill, LuaEngine, LuaVmPool, LuaVmPoolConfig, LuaVmPoolState, LuaVmRequestScopeGuard,
    SkillConfigStore, VulcanInternalExecutionContext, default_runlua_vm_pool_config,
    get_vulcan_context_table, get_vulcan_deps_table, get_vulcan_runtime_internal_table,
    get_vulcan_table, json_to_lua_table, normalize_host_visible_path_text,
    populate_vulcan_dependency_context, populate_vulcan_file_context,
    populate_vulcan_internal_execution_context, render_host_visible_path,
};
use crate::host::callbacks::runtime_model_callback_test_guard;
use crate::host::database::RuntimeDatabaseProviderCallbacks;
use crate::lua_skill::SkillMeta;
use crate::runtime::encoding::{RuntimeTextEncoding, encode_runtime_text};
use crate::runtime_options::LuaRuntimeRunLuaPoolConfig;
use crate::{
    LuaEngineOptions, LuaRuntimeHostOptions, RuntimeClientInfo, RuntimeHostToolAction,
    RuntimeModelEmbedRequest, RuntimeModelEmbedResponse, RuntimeModelError, RuntimeModelErrorCode,
    RuntimeModelLlmRequest, RuntimeModelLlmResponse, RuntimeModelUsage, RuntimeRequestContext,
    RuntimeSkillRoot, SkillInstallRequest, SkillInstallSourceType, SkillManagementAuthority,
    SkillUninstallOptions, set_host_tool_callback, set_model_embed_callback,
    set_model_llm_callback,
};
use mlua::{Table, Value as LuaValue};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

/// Guard one process-wide host-tool callback test and clear global callback state on drop.
/// 保护单个进程级宿主工具回调测试，并在释放时清理全局回调状态。
struct HostToolCallbackTestGuard {
    /// Hold the process-wide mutex guard until the current test finishes.
    /// 持有进程级互斥锁直到当前测试结束。
    _guard: MutexGuard<'static, ()>,
}

impl Drop for HostToolCallbackTestGuard {
    /// Clear the global host-tool callback when one guarded test finishes.
    /// 当受保护测试结束时清理全局宿主工具回调。
    fn drop(&mut self) {
        set_host_tool_callback(None);
    }
}

/// Acquire the process-wide host-tool callback test guard.
/// 获取进程级宿主工具回调测试保护锁。
fn host_tool_callback_test_guard() -> HostToolCallbackTestGuard {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("lock host tool callback test guard");
    set_host_tool_callback(None);
    HostToolCallbackTestGuard { _guard: guard }
}

/// Build one minimal loaded skill for collision-index tests.
/// 为冲突编号测试构造一个最小已加载 skill。
fn make_loaded_skill(
    directory_name: &str,
    skill_id: &str,
    local_entry_name: &str,
    lua_module: &str,
) -> LoadedSkill {
    let mut meta: SkillMeta = serde_yaml::from_str(&format!("name: {skill_id}\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: {local_entry_name}\n    lua_entry: runtime/test.lua\n    lua_module: {lua_module}\n"))
            .expect("deserialize minimal skill meta");
    meta.bind_directory_skill_id(skill_id.to_string());
    LoadedSkill {
        meta,
        dir: PathBuf::from(format!("D:/tests/{directory_name}")),
        root_name: "ROOT".to_string(),
        lancedb_binding: None,
        sqlite_binding: None,
        resolved_entry_names: HashMap::new(),
    }
}

/// Verify host-visible path normalization strips the Windows drive-letter verbatim prefix.
/// 验证对宿主可见的路径归一化会去掉 Windows 盘符 verbatim 前缀。
#[cfg(windows)]
#[test]
fn normalize_host_visible_path_text_strips_windows_drive_verbatim_prefix() {
    assert_eq!(
        normalize_host_visible_path_text(r"\\?\C:\runtime-test-root\skill.lua"),
        r"C:\runtime-test-root\skill.lua"
    );
}

/// Verify host-visible path normalization strips the Windows UNC verbatim prefix.
/// 验证对宿主可见的路径归一化会去掉 Windows UNC verbatim 前缀。
#[cfg(windows)]
#[test]
fn normalize_host_visible_path_text_strips_windows_unc_verbatim_prefix() {
    assert_eq!(
        normalize_host_visible_path_text(r"\\?\UNC\server\share\skill.lua"),
        r"\\server\share\skill.lua"
    );
}

/// Build one minimal engine instance used only for registry tests.
/// 构造仅用于入口注册表测试的最小引擎实例。
fn make_test_engine(skills: HashMap<String, LoadedSkill>) -> LuaEngine {
    LuaEngine {
        skills,
        entry_registry: Default::default(),
        runtime_skill_roots: Vec::new(),
        pool: Arc::new(LuaVmPool {
            config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
            state: Mutex::new(LuaVmPoolState {
                available: Vec::new(),
                total_count: 0,
            }),
            condvar: Condvar::new(),
        }),
        runlua_pool: Arc::new(LuaVmPool::new(default_runlua_vm_pool_config())),
        runtime_sessions: Arc::new(RuntimeSessionManager::new()),
        skill_config_store: Arc::new(
            SkillConfigStore::new(None).expect("create runtime test skill config store"),
        ),
        lancedb_host: None,
        sqlite_host: None,
        database_provider_callbacks: Arc::new(RuntimeDatabaseProviderCallbacks::default()),
        host_options: Arc::new(LuaRuntimeHostOptions::default()),
    }
}

/// Build one minimal runtime engine that can execute pooled-VM isolation tests.
/// 构造一个可用于池化虚拟机隔离测试的最小运行时引擎。
fn make_runtime_test_engine() -> LuaEngine {
    make_runtime_test_engine_with_host_options(LuaRuntimeHostOptions::default())
}

/// Build one minimal runtime engine with explicit host options.
/// 使用显式宿主选项构造一个最小运行时引擎。
fn make_runtime_test_engine_with_host_options(host_options: LuaRuntimeHostOptions) -> LuaEngine {
    LuaEngine::new(LuaEngineOptions {
        host_options,
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create runtime test engine")
}

/// Build one temporary runtime root path for one isolated skill-config test case.
/// 为单个隔离技能配置测试用例构造一条临时运行时根目录路径。
fn make_temp_runtime_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "luaskills_{}_{}_{}",
        label,
        std::process::id(),
        label.len()
    ))
}

/// Build one stable absolute file path string for payload-validation tests.
/// 为载荷校验测试构造一条稳定绝对文件路径字符串。
fn make_change_set_test_path(file_name: &str) -> String {
    render_host_visible_path(&std::env::temp_dir().join(file_name))
}

/// Create one minimal runtime directory layout used by skill-config tests.
/// 创建技能配置测试使用的最小运行时目录结构。
fn create_runtime_test_layout(runtime_root: &Path) {
    for relative_path in [
        "skills",
        "temp",
        "resources",
        "lua_packages",
        "bin/tools",
        "libs",
    ] {
        fs::create_dir_all(runtime_root.join(relative_path))
            .expect("create runtime test layout path");
    }
}

/// Write one minimal packaged-runtime luaskills-packages metadata tree for runtime validation tests.
/// 为运行时校验测试写入一个最小打包运行时 luaskills-packages 元数据目录树。
fn write_runtime_packages_test_metadata(runtime_root: &Path) {
    let resources_dir = runtime_root.join("resources");
    let packages_root = resources_dir.join("luaskills-packages");
    let help_packages_dir = packages_root.join("help").join("packages");
    let help_modules_dir = packages_root.join("help").join("modules");
    let packages_licenses_dir = runtime_root.join("licenses").join("luaskills-packages");
    fs::create_dir_all(&help_packages_dir).expect("create package help test dir");
    fs::create_dir_all(&help_modules_dir).expect("create module help test dir");
    fs::create_dir_all(&packages_licenses_dir).expect("create package license test dir");

    fs::write(
        resources_dir.join("lua-runtime-manifest.json"),
        "{\n  \"schema_version\": 1,\n  \"layout\": \"luaskills-runtime-v1\"\n}\n",
    )
    .expect("write runtime manifest test file");
    fs::write(
        packages_root.join("lua_packages.txt"),
        "pkg demo-package 0.1.0\n",
    )
    .expect("write package compatibility file");
    fs::write(
        packages_root.join("install-manifest.json"),
        "{\n  \"schema_version\": 1,\n  \"packages\": []\n}\n",
    )
    .expect("write package install manifest");
    fs::write(
            packages_root.join("platform-support.json"),
            "{\n  \"schema_version\": 1,\n  \"supported_targets\": [\"windows-x64\", \"linux-x64\", \"linux-arm64\", \"macos-x64\", \"macos-arm64\"]\n}\n",
        )
        .expect("write package platform support");
    fs::write(
        packages_root.join("THIRD_PARTY_LICENSES.json"),
        "{\n  \"schema_version\": 1,\n  \"luarocks_packages\": []\n}\n",
    )
    .expect("write package third-party licenses");
    fs::write(
        packages_root.join("THIRD_PARTY_NOTICES.md"),
        "# Third-Party Notices\n",
    )
    .expect("write package third-party notices");
    fs::write(
        packages_root.join("help").join("index.json"),
        "{\n  \"schema_version\": 1,\n  \"packages\": [],\n  \"modules\": []\n}\n",
    )
    .expect("write package help index");
    fs::write(
        help_packages_dir.join("demo-package.json"),
        "{\n  \"schema_version\": 1,\n  \"package_name\": \"demo-package\"\n}\n",
    )
    .expect("write package help document");
    fs::write(
        packages_licenses_dir.join("index.json"),
        "{\n  \"schema_version\": 1,\n  \"luarocks_packages\": []\n}\n",
    )
    .expect("write package license index");
    fs::write(
            resources_dir.join("luaskills-packages-manifest.json"),
            "{\n  \"schema_version\": 1,\n  \"layout\": \"luaskills-packages-runtime-v1\",\n  \"paths\": {\n    \"install_manifest\": \"resources/luaskills-packages/install-manifest.json\",\n    \"compat_lua_packages_txt\": \"resources/luaskills-packages/lua_packages.txt\",\n    \"platform_support\": \"resources/luaskills-packages/platform-support.json\",\n    \"third_party_licenses\": \"resources/luaskills-packages/THIRD_PARTY_LICENSES.json\",\n    \"third_party_notices\": \"resources/luaskills-packages/THIRD_PARTY_NOTICES.md\",\n    \"help_index\": \"resources/luaskills-packages/help/index.json\",\n    \"package_help_root\": \"resources/luaskills-packages/help/packages\",\n    \"module_help_root\": \"resources/luaskills-packages/help/modules\",\n    \"license_index\": \"licenses/luaskills-packages/index.json\"\n  }\n}\n",
        )
        .expect("write runtime packages manifest");
}

/// Write one minimal skill fixture that reads one value from `vulcan.config`.
/// 写入一个最小技能夹具，用于从 `vulcan.config` 读取单个值。
fn write_skill_config_test_skill(runtime_root: &Path, skill_id: &str) -> PathBuf {
    let skill_dir = runtime_root.join("skills").join(skill_id);
    fs::create_dir_all(skill_dir.join("runtime")).expect("create config test runtime dir");
    fs::write(
            skill_dir.join("skill.yaml"),
            format!(
                "name: {skill_id}\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: ping\n    description: Config ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: {skill_id}.ping\n"
            ),
        )
        .expect("write config test skill yaml");
    fs::write(
            skill_dir.join("runtime").join("ping.lua"),
            "return function(args)\n  local value = vulcan.config.get(\"api_token\")\n  if value == nil then\n    return \"missing\"\n  end\n  return value\nend\n",
        )
        .expect("write config test runtime entry");
    skill_dir
}

/// Write one minimal enabled skill fixture into a specific skills root.
/// 将一个最小启用技能夹具写入指定 skills 根目录。
fn write_minimal_skill_to_root(skill_root: &Path, skill_id: &str) -> PathBuf {
    write_minimal_skill_to_root_with_response(skill_root, skill_id, "ok")
}

/// Write one minimal enabled skill fixture with a deterministic response into a specific skills root.
/// 将带有确定响应的最小启用技能夹具写入指定 skills 根目录。
fn write_minimal_skill_to_root_with_response(
    skill_root: &Path,
    skill_id: &str,
    response: &str,
) -> PathBuf {
    let skill_dir = skill_root.join(skill_id);
    fs::create_dir_all(skill_dir.join("runtime")).expect("create minimal skill runtime dir");
    fs::write(
            skill_dir.join("skill.yaml"),
            format!(
                "name: {skill_id}\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: ping\n    description: Minimal ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: {skill_id}.ping\n"
            ),
        )
        .expect("write minimal skill yaml");
    fs::write(
        skill_dir.join("runtime").join("ping.lua"),
        format!("return function(args)\n  return '{response}'\nend\n"),
    )
    .expect("write minimal skill runtime entry");
    skill_dir
}

/// Write one model-capability test skill with caller-provided Lua source.
/// 写入一个使用调用方提供 Lua 源码的模型能力测试 skill。
fn write_model_test_skill_to_root(skill_root: &Path, skill_id: &str, lua_source: &str) -> PathBuf {
    let skill_dir = skill_root.join(skill_id);
    fs::create_dir_all(skill_dir.join("runtime")).expect("create model test skill runtime dir");
    fs::write(
            skill_dir.join("skill.yaml"),
            format!(
                "name: {skill_id}\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: ping\n    description: Model test entry.\n    lua_entry: runtime/ping.lua\n    lua_module: {skill_id}.ping\n"
            ),
        )
        .expect("write model test skill yaml");
    fs::write(skill_dir.join("runtime").join("ping.lua"), lua_source)
        .expect("write model test runtime entry");
    skill_dir
}

/// Write one skill fixture whose final AI-facing input schema comes from one external JSON file.
/// 写入一个最终面向 AI 输入 schema 来自外部 JSON 文件的技能夹具。
fn write_schema_file_skill_to_root(skill_root: &Path, skill_id: &str) -> PathBuf {
    let skill_dir = skill_root.join(skill_id);
    fs::create_dir_all(skill_dir.join("runtime")).expect("create schema skill runtime dir");
    fs::create_dir_all(skill_dir.join("schemas")).expect("create schema skill schema dir");
    fs::write(
        skill_dir.join("skill.yaml"),
        format!(
            "name: {skill_id}\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: inspect\n    description: Schema file entry.\n    lua_entry: runtime/inspect.lua\n    lua_module: {skill_id}.inspect\n    input_schema_file: schemas/inspect.input.schema.json\n"
        ),
    )
    .expect("write schema skill yaml");
    fs::write(
        skill_dir.join("schemas").join("inspect.input.schema.json"),
        serde_json::to_string_pretty(&json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "nodes": {
                    "type": "array",
                    "description": "Node selector list.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "file": { "type": "string" },
                            "structural_path": { "type": "string" }
                        },
                        "required": ["file", "structural_path"]
                    }
                },
                "strict": {
                    "type": "boolean",
                    "description": "Enable strict validation."
                }
            },
            "required": ["nodes"]
        }))
        .expect("serialize schema skill input schema"),
    )
    .expect("write schema skill input schema");
    fs::write(
        skill_dir.join("runtime").join("inspect.lua"),
        "return function(args)\n  return 'schema-ok'\nend\n",
    )
    .expect("write schema skill runtime entry");
    skill_dir
}

/// Verify runtime entry export carries the resolved external JSON input schema and derived parameters.
/// 验证运行时入口导出会携带已解析的外部 JSON 输入 schema 与推导出的参数列表。
#[test]
fn list_entries_exposes_resolved_entry_input_schema() {
    let runtime_root = make_temp_runtime_root("entry-input-schema-export");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);
    write_schema_file_skill_to_root(&runtime_root.join("skills"), "demo-schema-skill");

    let mut engine = LuaEngine::new(LuaEngineOptions::new(
        LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 30,
        },
        LuaRuntimeHostOptions {
            temp_dir: Some(runtime_root.join("temp")),
            resources_dir: Some(runtime_root.join("resources")),
            lua_packages_dir: Some(runtime_root.join("lua_packages")),
            host_provided_tool_root: Some(runtime_root.join("bin").join("tools")),
            host_provided_lua_root: Some(runtime_root.join("lua_packages")),
            host_provided_ffi_root: Some(runtime_root.join("libs")),
            download_cache_root: Some(runtime_root.join("temp").join("downloads")),
            ..LuaRuntimeHostOptions::default()
        },
    ))
    .expect("create engine for schema export test");
    engine
        .load_from_roots(&[RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("skills"),
        }])
        .expect("load schema export test root");

    let entries = engine.list_entries();
    let entry = entries
        .iter()
        .find(|item| item.local_name == "inspect")
        .expect("inspect entry");
    assert_eq!(entry.input_schema["type"], "object");
    assert_eq!(entry.input_schema["required"], json!(["nodes"]));
    assert_eq!(entry.input_schema["properties"]["nodes"]["type"], "array");
    assert_eq!(
        entry.input_schema["properties"]["nodes"]["items"]["properties"]["file"]["type"],
        "string"
    );
    assert_eq!(entry.parameters.len(), 2);
    assert_eq!(entry.parameters[0].name, "nodes");
    assert_eq!(entry.parameters[0].param_type, "array");
    assert!(entry.parameters[0].required);
    assert_eq!(entry.parameters[1].name, "strict");
    assert_eq!(entry.parameters[1].param_type, "boolean");

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify the canonical `change_set` validator accepts explicit AI-oriented modify hunks and file lifecycle records.
/// 验证 canonical `change_set` 校验器会接受面向 AI 的显式 modify hunk 与文件生命周期记录。
#[test]
fn validate_change_set_payload_accepts_hunks_and_file_lifecycle_changes() {
    let modify_path = make_change_set_test_path("luaskills_change_set_modify.lua");
    let create_path = make_change_set_test_path("luaskills_change_set_create.lua");
    let delete_path = make_change_set_test_path("luaskills_change_set_delete.lua");
    let rename_old_path = make_change_set_test_path("luaskills_change_set_old.lua");
    let rename_new_path = make_change_set_test_path("luaskills_change_set_new.lua");
    let payload = json!({
        "mode": "applied",
        "summary": "Updated one file and lifecycle metadata.",
        "files": [
            {
                "change": "modify",
                "path": modify_path,
                "hunks": [
                    {
                        "before": "local a = 1\nlocal b = 2",
                        "delete": [
                            { "line": 10, "content": "local x = 1" },
                            { "line": 11, "content": "return x" }
                        ],
                        "insert": [
                            { "line": 10, "content": "local x = 2" },
                            { "line": 11, "content": "local y = 3" },
                            { "line": 12, "content": "return x + y" }
                        ],
                        "after": "end\nreturn M"
                    }
                ]
            },
            {
                "change": "create",
                "path": create_path,
                "content": "local M = {}\nreturn M\n"
            },
            {
                "change": "delete",
                "path": delete_path,
                "content": "return legacy\n"
            },
            {
                "change": "rename",
                "old_path": rename_old_path,
                "new_path": rename_new_path
            }
        ]
    });

    validate_change_set_payload("demo.skill", &payload)
        .expect("change_set payload should be accepted");
}

/// Verify modify file records must carry at least one non-empty hunk list.
/// 验证 modify 文件记录必须携带至少一个非空 hunk 列表。
#[test]
fn validate_change_set_payload_rejects_modify_without_hunks() {
    let modify_path = make_change_set_test_path("luaskills_change_set_modify_missing_hunks.lua");
    let payload = json!({
        "mode": "applied",
        "files": [
            {
                "change": "modify",
                "path": modify_path
            }
        ]
    });

    let error = validate_change_set_payload("demo.skill", &payload)
        .expect_err("modify file record should require hunks");
    assert!(error.contains("change_set.files[0].hunks"));
}

/// Verify modify hunks must carry at least one deleted or inserted line block.
/// 验证 modify hunk 必须至少携带一组删除或插入行块。
#[test]
fn validate_change_set_payload_rejects_empty_modify_hunk() {
    let modify_path = make_change_set_test_path("luaskills_change_set_modify_empty_hunk.lua");
    let payload = json!({
        "mode": "applied",
        "files": [
            {
                "change": "modify",
                "path": modify_path,
                "hunks": [
                    {
                        "before": "",
                        "delete": [],
                        "insert": [],
                        "after": ""
                    }
                ]
            }
        ]
    });

    let error = validate_change_set_payload("demo.skill", &payload)
        .expect_err("modify hunk should require deleted or inserted lines");
    assert!(error.contains("must include at least one deleted or inserted line"));
}

/// Verify rename records must expose both old and new absolute file paths.
/// 验证 rename 记录必须同时暴露旧绝对路径与新绝对路径。
#[test]
fn validate_change_set_payload_rejects_rename_without_both_paths() {
    let rename_old_path = make_change_set_test_path("luaskills_change_set_old_only.lua");
    let payload = json!({
        "mode": "applied",
        "files": [
            {
                "change": "rename",
                "old_path": rename_old_path
            }
        ]
    });

    let error = validate_change_set_payload("demo.skill", &payload)
        .expect_err("rename record should require both old_path and new_path");
    assert!(error.contains("change_set.files[0].new_path"));
}

/// Verify modify line blocks must keep ascending line numbers so hosts and models can replay them deterministically.
/// 验证 modify 行块必须保持递增行号，确保宿主与模型可以确定性回放。
#[test]
fn validate_change_set_payload_rejects_out_of_order_hunk_lines() {
    let modify_path = make_change_set_test_path("luaskills_change_set_modify_unordered_lines.lua");
    let payload = json!({
        "mode": "applied",
        "files": [
            {
                "change": "modify",
                "path": modify_path,
                "hunks": [
                    {
                        "before": "local a = 1",
                        "delete": [
                            { "line": 11, "content": "return x" },
                            { "line": 10, "content": "local x = 1" }
                        ],
                        "insert": [],
                        "after": "return M"
                    }
                ]
            }
        ]
    });

    let error = validate_change_set_payload("demo.skill", &payload)
        .expect_err("modify hunk line numbers should be strictly increasing");
    assert!(error.contains("line numbers must be strictly increasing"));
}

/// Verify ROOT keeps priority over PROJECT and USER for identical skill ids.
/// 验证 ROOT 对同名 skill 始终高于 PROJECT 与 USER。
#[test]
fn load_from_roots_keeps_root_priority_over_project_and_user() {
    let runtime_root = make_temp_runtime_root("formal-root-load-priority");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let project_root = RuntimeSkillRoot {
        name: "PROJECT".to_string(),
        skills_dir: runtime_root.join("project_skills"),
    };
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    write_minimal_skill_to_root_with_response(&root_root.skills_dir, "vulcan-codekit", "root");
    write_minimal_skill_to_root_with_response(
        &project_root.skills_dir,
        "vulcan-codekit",
        "project",
    );
    write_minimal_skill_to_root_with_response(&user_root.skills_dir, "vulcan-codekit", "user");
    let mut engine = make_runtime_test_engine();
    engine
        .load_from_roots(&[root_root, project_root, user_root])
        .expect("formal root chain should load");

    let result = engine
        .call_skill("vulcan-codekit-ping", &json!({}), None)
        .expect("call root-priority skill");
    assert_eq!(result.content, "root");

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify one packaged runtime loads successfully when the embedded luaskills-packages metadata tree is complete.
/// 验证在内嵌 luaskills-packages 元数据目录树完整时，一个打包运行时能够成功加载。
#[test]
fn load_from_roots_accepts_packaged_runtime_with_packages_metadata() {
    let runtime_root = make_temp_runtime_root("packaged-runtime-packages-ok");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);
    write_runtime_packages_test_metadata(&runtime_root);
    write_minimal_skill_to_root(&runtime_root.join("skills"), "demo-packaged-skill");

    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.resources_dir = Some(runtime_root.join("resources"));
    host_options.lua_packages_dir = Some(runtime_root.join("lua_packages"));
    host_options.host_provided_lua_root = Some(runtime_root.join("lua_packages"));
    let mut engine = make_runtime_test_engine_with_host_options(host_options);
    engine
        .load_from_roots(&[RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("skills"),
        }])
        .expect("packaged runtime with package metadata should load");

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify one packaged runtime fails with a clear error when the top-level luaskills-packages manifest is missing.
/// 验证当顶层 luaskills-packages 清单缺失时，一个打包运行时会给出清晰错误并加载失败。
#[test]
fn load_from_roots_rejects_packaged_runtime_without_packages_manifest() {
    let runtime_root = make_temp_runtime_root("packaged-runtime-missing-manifest");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);
    fs::write(
        runtime_root
            .join("resources")
            .join("lua-runtime-manifest.json"),
        "{\n  \"schema_version\": 1,\n  \"layout\": \"luaskills-runtime-v1\"\n}\n",
    )
    .expect("write runtime manifest trigger file");
    write_minimal_skill_to_root(&runtime_root.join("skills"), "demo-missing-manifest");

    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.resources_dir = Some(runtime_root.join("resources"));
    host_options.lua_packages_dir = Some(runtime_root.join("lua_packages"));
    host_options.host_provided_lua_root = Some(runtime_root.join("lua_packages"));
    let mut engine = make_runtime_test_engine_with_host_options(host_options);
    let error_text = engine
        .load_from_roots(&[RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("skills"),
        }])
        .expect_err("packaged runtime without package manifest should fail")
        .to_string();
    assert!(
        error_text.contains("luaskills-packages-manifest.json"),
        "unexpected error text: {}",
        error_text
    );

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify one packaged runtime fails with a clear error when one manifest-declared packages file is missing.
/// 验证当清单声明的某个 packages 文件缺失时，一个打包运行时会给出清晰错误并加载失败。
#[test]
fn load_from_roots_rejects_packaged_runtime_when_declared_packages_file_is_missing() {
    let runtime_root = make_temp_runtime_root("packaged-runtime-missing-help-index");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);
    write_runtime_packages_test_metadata(&runtime_root);
    fs::remove_file(
        runtime_root
            .join("resources")
            .join("luaskills-packages")
            .join("help")
            .join("index.json"),
    )
    .expect("remove package help index");
    write_minimal_skill_to_root(&runtime_root.join("skills"), "demo-missing-help-index");

    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.resources_dir = Some(runtime_root.join("resources"));
    host_options.lua_packages_dir = Some(runtime_root.join("lua_packages"));
    host_options.host_provided_lua_root = Some(runtime_root.join("lua_packages"));
    let mut engine = make_runtime_test_engine_with_host_options(host_options);
    let error_text = engine
        .load_from_roots(&[RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("skills"),
        }])
        .expect_err("packaged runtime with missing declared file should fail")
        .to_string();
    assert!(
        error_text.contains("luaskills-packages\\help\\index.json")
            || error_text.contains("luaskills-packages/help/index.json"),
        "unexpected error text: {}",
        error_text
    );

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify delegated query helpers hide ROOT-owned metadata while runtime calls still use active skills.
/// 验证委托查询辅助函数会隐藏 ROOT 元数据，同时运行时调用仍使用已激活技能。
#[test]
fn delegated_authority_query_helpers_hide_root_skills() {
    let runtime_root = make_temp_runtime_root("delegated-query-hides-root");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: " root ".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    write_minimal_skill_to_root(&root_root.skills_dir, "vulcan-root-skill");
    write_minimal_skill_to_root(&user_root.skills_dir, "vulcan-user-skill");
    let mut engine = make_runtime_test_engine();
    engine
        .load_from_roots(&[root_root, user_root])
        .expect("root and user runtime should load");

    let system_entries = engine.list_entries_for_authority(SkillManagementAuthority::System);
    let delegated_entries =
        engine.list_entries_for_authority(SkillManagementAuthority::DelegatedTool);
    assert!(
        system_entries
            .iter()
            .any(|entry| entry.root_name == " root ")
    );
    assert!(
        delegated_entries
            .iter()
            .all(|entry| entry.root_name.trim().to_ascii_uppercase() != "ROOT")
    );

    let system_help = engine.list_skill_help_for_authority(SkillManagementAuthority::System);
    let delegated_help =
        engine.list_skill_help_for_authority(SkillManagementAuthority::DelegatedTool);
    assert!(system_help.iter().any(|help| help.root_name == " root "));
    assert!(
        delegated_help
            .iter()
            .all(|help| help.root_name.trim().to_ascii_uppercase() != "ROOT")
    );

    let delegated_detail = engine
        .render_skill_help_detail_for_authority(
            SkillManagementAuthority::DelegatedTool,
            "vulcan-root-skill",
            "main",
            None,
        )
        .expect("delegated detail should be filtered");
    assert!(delegated_detail.is_none());

    let root_call = engine
        .call_skill("vulcan-root-skill-ping", &json!({}), None)
        .expect("runtime call should reach any active skill");
    assert_eq!(root_call.content, "ok");

    let root_run_lua = engine
        .run_lua(
            "return vulcan.call('vulcan-root-skill-ping', {})",
            &json!({}),
            None,
        )
        .expect("runtime Lua execution should use the active runtime view");
    assert_eq!(root_run_lua, json!("ok"));

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify formal root chains reject unknown labels and reversed priority order.
/// 验证正式根链会拒绝未知标签和反向优先级顺序。
#[test]
fn load_from_roots_rejects_unknown_or_reversed_formal_layers() {
    let runtime_root = make_temp_runtime_root("formal-root-chain-validation");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let mut engine = make_runtime_test_engine();
    let reversed_error = engine
        .load_from_roots(&[
            RuntimeSkillRoot {
                name: "USER".to_string(),
                skills_dir: runtime_root.join("user_skills"),
            },
            RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("root_skills"),
            },
        ])
        .expect_err("reversed formal root order should fail");
    assert!(
        reversed_error
            .to_string()
            .contains("ROOT -> PROJECT -> USER")
    );

    let unknown_error = engine
        .load_from_roots(&[RuntimeSkillRoot {
            name: "WORKSPACE".to_string(),
            skills_dir: runtime_root.join("workspace_skills"),
        }])
        .expect_err("unknown formal root label should fail");
    assert!(
        unknown_error
            .to_string()
            .contains("unsupported skill root label")
    );

    let missing_root_error = engine
        .load_from_roots(&[RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        }])
        .expect_err("missing ROOT layer should fail");
    assert!(
        missing_root_error
            .to_string()
            .contains("ROOT skill root is required")
    );

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify ordinary skills installs do not fall back to the system-controlled ROOT layer.
/// 验证普通 skills 安装不会回落到系统控制的 ROOT 层。
#[test]
fn install_skill_rejects_root_only_runtime() {
    let runtime_root = make_temp_runtime_root("ordinary-install-root-only");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
    let mut engine = make_runtime_test_engine();

    let error = engine
        .install_skill(
            &[root_root],
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("ordinary install must reject root-only runtime");
    assert!(error.to_string().contains("ROOT is system-controlled"));

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify system installs do not fall back to ordinary layers when ROOT is absent.
/// 验证 system 安装在缺少 ROOT 时不会回退到普通层。
#[test]
fn system_install_skill_rejects_runtime_without_root() {
    let runtime_root = make_temp_runtime_root("system-install-without-root");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
    let mut engine = make_runtime_test_engine();

    let error = engine
        .system_install_skill(
            &[user_root],
            SkillManagementAuthority::System,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("system install without ROOT should fail");
    assert!(error.to_string().contains("ROOT skill root is required"));

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify the Lua-visible ordinary skill-management layer list excludes ROOT.
/// 验证 Lua 可见的普通技能管理层级列表不包含 ROOT。
#[test]
fn runtime_skills_layers_excludes_root() {
    let runtime_root = make_temp_runtime_root("runtime-skills-layers-root-only");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.capabilities.enable_skill_management_bridge = true;
    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options,
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create root-only layer test engine");
    engine
        .load_from_roots(&[root_root])
        .expect("root-only runtime should load");
    let result = engine
        .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
        .expect("layers function should run");

    assert_eq!(result["labels"], json!([]));
    assert_eq!(result["layers"], json!([]));
    assert_eq!(result["writable"], json!(false));
    assert!(result["default"].is_null());

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify layers reflects loaded PROJECT and USER roots and the bridge writable policy.
/// 验证 layers 会反映已加载 PROJECT/USER 根以及桥接写入策略。
#[test]
fn runtime_skills_layers_reflects_loaded_roots_and_bridge_policy() {
    let runtime_root = make_temp_runtime_root("runtime-skills-layers-dynamic");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    let mut engine = make_runtime_test_engine();
    engine
        .load_from_roots(&[root_root.clone(), user_root])
        .expect("root and user runtime should load");
    let disabled_result = engine
        .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
        .expect("layers function should run when bridge is disabled");
    assert_eq!(disabled_result["default"], json!("USER"));
    assert_eq!(disabled_result["labels"], json!(["USER"]));
    assert_eq!(disabled_result["writable"], json!(false));
    assert_eq!(disabled_result["layers"][0]["writable"], json!(false));

    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.capabilities.enable_skill_management_bridge = true;
    let mut enabled_engine = LuaEngine::new(LuaEngineOptions {
        host_options,
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create enabled layer test engine");
    let project_root = RuntimeSkillRoot {
        name: "PROJECT".to_string(),
        skills_dir: runtime_root.join("project_skills"),
    };
    enabled_engine
        .load_from_roots(&[
            root_root,
            project_root,
            RuntimeSkillRoot {
                name: "USER".to_string(),
                skills_dir: runtime_root.join("enabled_user_skills"),
            },
        ])
        .expect("root, project, user runtime should load");
    let enabled_result = enabled_engine
        .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
        .expect("layers function should run when bridge is enabled");
    assert_eq!(enabled_result["default"], json!("USER"));
    assert_eq!(enabled_result["labels"], json!(["PROJECT", "USER"]));
    assert_eq!(enabled_result["writable"], json!(true));
    assert_eq!(enabled_result["layers"][0]["writable"], json!(true));
    assert!(
        enabled_result["labels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value != "ROOT")
    );

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify the ordinary Lua bridge rejects ROOT targets before dispatching to the host callback.
/// 验证普通 Lua 桥接会在分发到宿主回调前拒绝 ROOT 目标。
#[test]
fn runtime_skills_bridge_rejects_root_payload_before_callback() {
    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.capabilities.enable_skill_management_bridge = true;
    let engine = LuaEngine::new(LuaEngineOptions {
        host_options,
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create bridge test engine");

    let error = engine
        .run_lua(
            "return vulcan.runtime.skills.install({ layer = 'ROOT', skill_id = 'vulcan-codekit' })",
            &json!({}),
            None,
        )
        .expect_err("root target should be rejected by bridge");
    assert!(error.contains("cannot target the system-controlled ROOT layer"));
    assert!(!error.contains("no host callback"));

    let object_error = engine
            .run_lua(
                "return vulcan.runtime.skills.install({ target_root = { name = 'ROOT', skills_dir = 'C:/tmp/root-skills' }, skill_id = 'vulcan-codekit' })",
                &json!({}),
                None,
            )
            .expect_err("root target object should be rejected by bridge");
    assert!(object_error.contains("cannot target the system-controlled ROOT layer"));
    assert!(!object_error.contains("no host callback"));
}

/// Verify ordinary explicit-root APIs reject ROOT write targets before lifecycle work starts.
/// 验证普通显式根 API 会在生命周期工作开始前拒绝 ROOT 写入目标。
#[test]
fn ordinary_explicit_root_apis_reject_root_target() {
    let runtime_root = make_temp_runtime_root("ordinary-explicit-root-rejects-root");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
    fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
    let skill_roots = vec![root_root.clone(), user_root];
    let mut engine = make_runtime_test_engine();

    let error = engine
        .install_skill_in_root(
            &skill_roots,
            &root_root,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("ordinary explicit root install should reject ROOT");
    assert!(error.to_string().contains("ordinary skills plane cannot"));

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify ROOT-owned skill ids cannot be installed or updated in ordinary layers by any authority.
/// 验证 ROOT 拥有的 skill id 不能被任何权限安装或更新到普通层。
#[test]
fn root_owned_skill_id_blocks_project_user_install_update_for_all_authorities() {
    let runtime_root = make_temp_runtime_root("root-owned-skill-id-blocks-ordinary");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let project_root = RuntimeSkillRoot {
        name: "PROJECT".to_string(),
        skills_dir: runtime_root.join("project_skills"),
    };
    write_minimal_skill_to_root(&root_root.skills_dir, "vulcan-codekit");
    write_minimal_skill_to_root(&project_root.skills_dir, "vulcan-codekit");
    let skill_roots = vec![root_root, project_root.clone()];
    let mut engine = make_runtime_test_engine();

    let ordinary_install_error = engine
        .install_skill_in_root(
            &skill_roots,
            &project_root,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("ordinary install must reject ROOT-owned skill id");
    assert!(
        ordinary_install_error
            .to_string()
            .contains("ROOT system layer")
    );

    let system_install_error = engine
        .system_install_skill_in_root(
            &skill_roots,
            &project_root,
            SkillManagementAuthority::System,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("system install must reject ROOT-owned skill id in PROJECT");
    assert!(
        system_install_error
            .to_string()
            .contains("ROOT system layer")
    );

    let system_update_error = engine
        .system_update_skill_in_root(
            &skill_roots,
            &project_root,
            SkillManagementAuthority::System,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("system update must also reject ROOT-owned skill id in PROJECT");
    assert!(
        system_update_error
            .to_string()
            .contains("ROOT system layer")
    );

    let delegated_update_error = engine
        .system_update_skill_in_root(
            &skill_roots,
            &project_root,
            SkillManagementAuthority::DelegatedTool,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("delegated update must reject ROOT-owned skill id in PROJECT");
    assert!(
        delegated_update_error
            .to_string()
            .contains("ROOT system layer")
    );

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify ordinary explicit-root uninstall may clean a USER residual shadowed by ROOT.
/// 验证普通显式根卸载可以清理被 ROOT 遮蔽的 USER 残留。
#[test]
fn ordinary_uninstall_in_root_cleans_user_residual_when_root_owns_same_skill_id() {
    let runtime_root = make_temp_runtime_root("ordinary-uninstall-cleans-root-shadow");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    let root_skill_dir = write_minimal_skill_to_root(&root_root.skills_dir, "vulcan-codekit");
    let user_skill_dir = write_minimal_skill_to_root(&user_root.skills_dir, "vulcan-codekit");
    let skill_roots = vec![root_root, user_root.clone()];
    let mut engine = make_runtime_test_engine();

    let result = engine
        .uninstall_skill_in_root(
            &skill_roots,
            &user_root,
            "vulcan-codekit",
            &SkillUninstallOptions::default(),
        )
        .expect("ordinary uninstall should clean USER residual");
    assert_eq!(result.skill_id, "vulcan-codekit");
    assert!(!user_skill_dir.exists());
    assert!(root_skill_dir.exists());

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify delegated authority cannot use a system explicit-root API to write ROOT.
/// 验证委托权限不能借助 system 显式根 API 写入 ROOT。
#[test]
fn delegated_authority_rejects_system_root_write() {
    let runtime_root = make_temp_runtime_root("delegated-system-root-write-reject");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
    let skill_roots = vec![root_root.clone()];
    let mut engine = make_runtime_test_engine();

    let error = engine
        .system_install_skill_in_root(
            &skill_roots,
            &root_root,
            SkillManagementAuthority::DelegatedTool,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("delegated authority must reject ROOT writes");
    assert!(error.to_string().contains("DelegatedTool authority"));

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify explicit-root system updates fail instead of returning a successful missing-skill result.
/// 验证显式根 system 更新在缺少目标技能时会失败，而不是返回成功的 missing-skill 结果。
#[test]
fn system_update_skill_in_root_missing_target_returns_error() {
    let runtime_root = make_temp_runtime_root("system-update-target-missing");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user").join("skills"),
    };
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root").join("skills"),
    };
    fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
    fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
    let skill_roots = vec![root_root, user_root.clone()];
    let mut engine = make_runtime_test_engine();

    let error = engine
        .system_update_skill_in_root(
            &skill_roots,
            &user_root,
            SkillManagementAuthority::System,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("missing explicit-root update target should fail");
    let rendered = error.to_string();

    assert!(rendered.contains("not installed in target root 'USER'"));
    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify explicit-root apply rejects PROJECT changes when ROOT owns the same skill id.
/// 验证明确定根应用会在 ROOT 拥有同名 skill 时拒绝 PROJECT 变更。
#[test]
fn system_update_skill_in_root_rejects_shadowed_fallback_target() {
    let runtime_root = make_temp_runtime_root("system-update-shadowed-root");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let project_root = RuntimeSkillRoot {
        name: "PROJECT".to_string(),
        skills_dir: runtime_root.join("project_skills"),
    };
    write_minimal_skill_to_root(&root_root.skills_dir, "vulcan-codekit");
    write_minimal_skill_to_root(&project_root.skills_dir, "vulcan-codekit");
    let skill_roots = vec![root_root, project_root.clone()];
    let mut engine = make_runtime_test_engine();

    let error = engine
        .system_update_skill_in_root(
            &skill_roots,
            &project_root,
            SkillManagementAuthority::System,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("shadowed fallback target should fail before update");
    let rendered = error.to_string();

    assert!(rendered.contains("ROOT system layer"));
    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify explicit-root install derives skill ids with the same GitHub locator rules as the manager.
/// 验证明确定根安装使用与管理器一致的 GitHub 定位规则推导技能标识。
#[test]
fn system_install_skill_in_root_accepts_trailing_slash_github_url_for_shadow_check() {
    let runtime_root = make_temp_runtime_root("system-install-trailing-slash-source");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    write_minimal_skill_to_root(&user_root.skills_dir, "vulcan-codekit");
    fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
    let skill_roots = vec![root_root.clone(), user_root];
    let mut engine = make_runtime_test_engine();

    let error = engine
        .system_install_skill_in_root(
            &skill_roots,
            &root_root,
            SkillManagementAuthority::System,
            &SkillInstallRequest {
                skill_id: None,
                source: Some("https://github.com/LuaSkills/vulcan-codekit/".to_string()),
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("root install should derive source skill id before managed download");
    let rendered = error.to_string();

    assert!(!rendered.contains("shadowed by higher-priority root"));
    assert!(!rendered.contains("requires skill_id"));
    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify explicit-root system updates reject unlisted targets before probing target contents.
/// 验证明确定根 system 更新会在探测目标内容前拒绝链外目标。
#[test]
fn system_update_skill_in_root_rejects_unlisted_target_before_missing_target() {
    let runtime_root = make_temp_runtime_root("system-update-unlisted-root");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let rogue_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("rogue_skills"),
    };
    fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
    fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
    let skill_roots = vec![root_root, user_root];
    let mut engine = make_runtime_test_engine();

    let error = engine
        .system_update_skill_in_root(
            &skill_roots,
            &rogue_root,
            SkillManagementAuthority::System,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("unlisted explicit update target root should be rejected");
    let rendered = error.to_string();

    assert!(rendered.contains("not part of the full runtime root chain"));
    assert!(!rendered.contains("not installed in target root"));
    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify explicit-root uninstall rejects target roots outside the active runtime chain.
/// 验证明确定根卸载会拒绝当前运行时根链之外的目标根。
#[test]
fn system_uninstall_skill_in_root_rejects_unlisted_target_root() {
    let runtime_root = make_temp_runtime_root("system-uninstall-unlisted-root");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user").join("skills"),
    };
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root").join("skills"),
    };
    let rogue_root = RuntimeSkillRoot {
        name: "ROGUE".to_string(),
        skills_dir: runtime_root.join("rogue").join("skills"),
    };
    fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
    fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
    let rogue_skill_dir = write_minimal_skill_to_root(&rogue_root.skills_dir, "vulcan-codekit");
    let skill_roots = vec![root_root, user_root];
    let mut engine = make_runtime_test_engine();

    let error = engine
        .system_uninstall_skill_in_root(
            &skill_roots,
            &rogue_root,
            SkillManagementAuthority::System,
            "vulcan-codekit",
            &SkillUninstallOptions::default(),
        )
        .expect_err("unlisted explicit target root should be rejected");
    let rendered = error.to_string();

    assert!(rendered.contains("not part of the full runtime root chain"));
    assert!(
        rogue_skill_dir.exists(),
        "unlisted target skill directory should not be removed"
    );
    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify the isolated runlua pool uses the documented default sizing when the host does not override it.
/// 验证宿主未覆盖时隔离 runlua 池会使用文档声明的默认容量配置。
#[test]
fn runlua_pool_uses_default_config_when_host_does_not_override() {
    let engine = make_runtime_test_engine();
    assert_eq!(engine.runlua_pool.config.min_size, 1);
    assert_eq!(engine.runlua_pool.config.max_size, 4);
    assert_eq!(engine.runlua_pool.config.idle_ttl_secs, 60);
}

/// Verify the host can override the isolated runlua pool sizing with the same shape as the main VM pool.
/// 验证宿主可以使用与主虚拟机池相同的参数形状覆盖隔离 runlua 池容量。
#[test]
fn runlua_pool_honors_host_override_config() {
    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.runlua_pool_config = Some(LuaRuntimeRunLuaPoolConfig {
        min_size: 2,
        max_size: 5,
        idle_ttl_secs: 90,
    });
    let engine = LuaEngine::new(LuaEngineOptions {
        host_options,
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create runtime test engine with custom runlua pool");
    assert_eq!(engine.runlua_pool.config.min_size, 2);
    assert_eq!(engine.runlua_pool.config.max_size, 5);
    assert_eq!(engine.runlua_pool.config.idle_ttl_secs, 90);
}

/// Verify the engine host API persists string skill config values into one explicit config file.
/// 验证引擎宿主 API 会把字符串技能配置值持久化到显式配置文件中。
#[test]
fn skill_config_engine_api_persists_values_into_explicit_file() {
    let runtime_root = make_temp_runtime_root("skill_config_explicit_path");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);
    let config_file = runtime_root.join("custom").join("skill_config.json");

    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.skill_config_file_path = Some(config_file.clone());
    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options,
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create skill config test engine");

    engine
        .set_skill_config_value("demo-skill", "api_token", "sk-explicit")
        .expect("set explicit skill config");
    assert_eq!(
        engine
            .get_skill_config_value("demo-skill", "api_token")
            .expect("read explicit skill config"),
        Some("sk-explicit".to_string())
    );
    let entries = engine
        .list_skill_config_entries(Some("demo-skill"))
        .expect("list explicit skill config");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].skill_id, "demo-skill");
    assert_eq!(entries[0].key, "api_token");
    assert_eq!(entries[0].value, "sk-explicit");
    assert!(config_file.exists());

    let deleted = engine
        .delete_skill_config_value("demo-skill", "api_token")
        .expect("delete explicit skill config");
    assert!(deleted);
    assert_eq!(
        engine
            .get_skill_config_value("demo-skill", "api_token")
            .expect("read deleted explicit skill config"),
        None
    );

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify the unified skill config store falls back to `<runtime_root>/config/skill_config.json` after roots load.
/// 验证统一技能配置存储会在加载根目录后回退到 `<runtime_root>/config/skill_config.json`。
#[test]
fn skill_config_store_uses_default_runtime_config_file_after_load() {
    let runtime_root = make_temp_runtime_root("skill_config_default_path");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);

    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options: LuaRuntimeHostOptions::default(),
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create default skill config test engine");

    engine
        .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("skills"),
        }])
        .expect("load empty roots for default skill config path");

    let expected_path = runtime_root.join("config").join("skill_config.json");
    assert_eq!(
        engine
            .skill_config_store
            .file_path()
            .expect("resolve default skill config file path"),
        expected_path
    );

    engine
        .set_skill_config_value("demo-skill", "endpoint", "https://example.test")
        .expect("write default skill config");
    assert!(expected_path.exists());

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify the unified skill config store resolves the default config path even before the skills directory exists.
/// 验证统一技能配置存储会在技能目录尚未创建前解析默认配置路径。
#[test]
fn skill_config_store_initializes_default_path_before_skills_dir_exists() {
    let runtime_root = make_temp_runtime_root("skill_config_without_skills_dir");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    fs::create_dir_all(&runtime_root).expect("create runtime root without skills dir");

    let missing_skills_dir = runtime_root.join("skills");
    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options: LuaRuntimeHostOptions::default(),
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create config path initialization test engine");

    engine
        .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: missing_skills_dir,
        }])
        .expect("load roots without an existing skills directory");

    let expected_path = runtime_root.join("config").join("skill_config.json");
    assert_eq!(
        engine
            .skill_config_store
            .file_path()
            .expect("resolve config path without skills directory"),
        expected_path
    );

    engine
        .set_skill_config_value("demo-skill", "api_token", "sk-before-install")
        .expect("write config before any skills directory exists");
    assert!(expected_path.exists());

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify invalid reload requests fail before clearing the active runtime view.
/// 验证无效重载请求会在清空当前运行时视图前失败。
#[test]
fn reload_from_roots_rejects_invalid_chain_before_resetting_runtime_state() {
    let runtime_root = make_temp_runtime_root("reload-invalid-chain-preserves-state");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
    write_minimal_skill_to_root_with_response(&user_root.skills_dir, "vulcan-codekit", "user");
    let mut engine = make_runtime_test_engine();
    engine
        .load_from_roots(&[root_root, user_root.clone()])
        .expect("initial root and user runtime should load");

    let invalid_reload_error = engine
        .reload_from_roots(&[user_root])
        .expect_err("missing ROOT reload should fail");
    assert!(
        invalid_reload_error
            .to_string()
            .contains("ROOT skill root is required")
    );

    let result = engine
        .call_skill("vulcan-codekit-ping", &json!({}), None)
        .expect("old entry should remain callable after failed reload");
    assert_eq!(result.content, "user");

    let layers = engine
        .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
        .expect("layers should still use the previously loaded root chain");
    assert_eq!(layers["labels"], json!(["USER"]));
    assert_eq!(layers["default"], json!("USER"));

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify reload failures after formal validation still preserve the active runtime view.
/// 验证 formal 校验之后发生的重载失败仍会保留当前活动运行时视图。
#[test]
fn reload_from_roots_preserves_state_after_ambiguous_config_root_error() {
    let runtime_root = make_temp_runtime_root("reload-ambiguous-preserves-state");
    let first_ambiguous_root = make_temp_runtime_root("reload-ambiguous-first");
    let second_ambiguous_root = make_temp_runtime_root("reload-ambiguous-second");
    for path in [&runtime_root, &first_ambiguous_root, &second_ambiguous_root] {
        if path.exists() {
            let _ = fs::remove_dir_all(path);
        }
    }
    let root_root = RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: runtime_root.join("root_skills"),
    };
    let user_root = RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: runtime_root.join("user_skills"),
    };
    fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
    write_minimal_skill_to_root_with_response(&user_root.skills_dir, "vulcan-codekit", "user");
    let mut engine = make_runtime_test_engine();
    engine
        .load_from_roots(&[root_root, user_root])
        .expect("initial root and user runtime should load");
    let previous_config_path = engine
        .skill_config_store
        .file_path()
        .expect("resolve previous skill config path");

    let ambiguous_reload_error = engine
        .reload_from_roots(&[
            RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: first_ambiguous_root.join("skills"),
            },
            RuntimeSkillRoot {
                name: "PROJECT".to_string(),
                skills_dir: second_ambiguous_root.join("skills"),
            },
        ])
        .expect_err("ambiguous config root reload should fail");
    assert!(
        ambiguous_reload_error
            .to_string()
            .contains("multiple runtime roots map to different parents")
    );

    let result = engine
        .call_skill("vulcan-codekit-ping", &json!({}), None)
        .expect("old entry should remain callable after ambiguous reload failure");
    assert_eq!(result.content, "user");
    assert_eq!(
        engine
            .skill_config_store
            .file_path()
            .expect("resolve config path after failed reload"),
        previous_config_path
    );

    let layers = engine
        .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
        .expect("layers should still use the previous root chain");
    assert_eq!(layers["labels"], json!(["USER"]));
    assert_eq!(layers["default"], json!("USER"));

    let _ = fs::remove_dir_all(&runtime_root);
    let _ = fs::remove_dir_all(&first_ambiguous_root);
    let _ = fs::remove_dir_all(&second_ambiguous_root);
}

/// Verify reloading a different runtime root updates the default unified skill-config path.
/// 验证重新加载另一套运行时根目录时会同步更新默认统一技能配置路径。
#[test]
fn reload_from_roots_updates_default_skill_config_path() {
    let first_runtime_root = make_temp_runtime_root("skill_config_reload_first");
    let second_runtime_root = make_temp_runtime_root("skill_config_reload_second");
    if first_runtime_root.exists() {
        let _ = fs::remove_dir_all(&first_runtime_root);
    }
    if second_runtime_root.exists() {
        let _ = fs::remove_dir_all(&second_runtime_root);
    }
    create_runtime_test_layout(&first_runtime_root);
    create_runtime_test_layout(&second_runtime_root);

    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options: LuaRuntimeHostOptions::default(),
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create reload skill config test engine");

    engine
        .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: first_runtime_root.join("skills"),
        }])
        .expect("load first runtime root");
    assert_eq!(
        engine
            .skill_config_store
            .file_path()
            .expect("resolve first config path"),
        first_runtime_root.join("config").join("skill_config.json")
    );

    engine
        .reload_from_roots(&[crate::host::options::RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: second_runtime_root.join("skills"),
        }])
        .expect("reload second runtime root");
    assert_eq!(
        engine
            .skill_config_store
            .file_path()
            .expect("resolve second config path"),
        second_runtime_root.join("config").join("skill_config.json")
    );

    let _ = fs::remove_dir_all(&first_runtime_root);
    let _ = fs::remove_dir_all(&second_runtime_root);
}

/// Verify reload keeps the initially resolved explicit relative skill-config path.
/// 验证重载会保持初始解析后的显式相对技能配置路径。
#[test]
fn reload_from_roots_keeps_frozen_relative_explicit_skill_config_path() {
    let _cwd_guard = runlua_cwd_guard()
        .lock()
        .expect("lock cwd guard for explicit config reload test");
    let original_cwd = std::env::current_dir().expect("resolve original cwd");
    /// Restore the process current directory when the test exits.
    /// 在测试退出时恢复进程当前工作目录。
    struct CwdRestoreGuard(PathBuf);
    impl Drop for CwdRestoreGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }
    let _cwd_restore = CwdRestoreGuard(original_cwd);
    let first_cwd = make_temp_runtime_root("skill_config_reload_relative_cwd_first");
    let second_cwd = make_temp_runtime_root("skill_config_reload_relative_cwd_second");
    let runtime_root = make_temp_runtime_root("skill_config_reload_relative_runtime");
    for path in [&first_cwd, &second_cwd, &runtime_root] {
        if path.exists() {
            let _ = fs::remove_dir_all(path);
        }
        fs::create_dir_all(path).expect("create explicit config reload test directory");
    }
    let relative_config_path = PathBuf::from("config").join("skill_config.json");
    std::env::set_current_dir(&first_cwd).expect("switch to first cwd");
    let expected_config_path = first_cwd.join(&relative_config_path);

    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.skill_config_file_path = Some(relative_config_path);
    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options,
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create explicit relative config reload test engine");
    engine
        .load_from_roots(&[RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        }])
        .expect("load initial root for explicit relative config reload test");
    assert_eq!(
        engine
            .skill_config_store
            .file_path()
            .expect("resolve explicit config path before reload"),
        expected_config_path
    );

    std::env::set_current_dir(&second_cwd).expect("switch to second cwd before reload");
    engine
        .reload_from_roots(&[RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("other_root_skills"),
        }])
        .expect("reload should preserve frozen explicit config path");
    assert_eq!(
        engine
            .skill_config_store
            .file_path()
            .expect("resolve explicit config path after reload"),
        expected_config_path
    );

    let _ = fs::remove_dir_all(&first_cwd);
    let _ = fs::remove_dir_all(&second_cwd);
    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify explicit unified config file paths bypass ambiguous runtime-root inference.
/// 验证显式统一配置文件路径会绕过歧义运行时根目录推导。
#[test]
fn load_from_roots_accepts_explicit_skill_config_path_for_ambiguous_runtime_roots() {
    let first_runtime_root = make_temp_runtime_root("skill_config_explicit_ambiguous_first");
    let second_runtime_root = make_temp_runtime_root("skill_config_explicit_ambiguous_second");
    if first_runtime_root.exists() {
        let _ = fs::remove_dir_all(&first_runtime_root);
    }
    if second_runtime_root.exists() {
        let _ = fs::remove_dir_all(&second_runtime_root);
    }
    fs::create_dir_all(&first_runtime_root).expect("create first explicit ambiguous runtime root");
    fs::create_dir_all(&second_runtime_root)
        .expect("create second explicit ambiguous runtime root");
    let explicit_config_file = first_runtime_root.join("custom").join("skill_config.json");

    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.skill_config_file_path = Some(explicit_config_file.clone());
    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options,
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create explicit ambiguous root test engine");

    engine
        .load_from_roots(&[
            crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: first_runtime_root.join("skills"),
            },
            crate::host::options::RuntimeSkillRoot {
                name: "PROJECT".to_string(),
                skills_dir: second_runtime_root.join("skills"),
            },
        ])
        .expect("explicit config path should bypass ambiguous runtime roots");

    assert_eq!(
        engine
            .skill_config_store
            .file_path()
            .expect("resolve explicit config path"),
        explicit_config_file
    );

    let _ = fs::remove_dir_all(&first_runtime_root);
    let _ = fs::remove_dir_all(&second_runtime_root);
}

/// Verify divergent runtime roots require one explicit unified skill config file path.
/// 验证运行时根目录分叉时必须显式提供统一技能配置文件路径。
#[test]
fn load_from_roots_rejects_ambiguous_default_skill_config_runtime_root() {
    let first_runtime_root = make_temp_runtime_root("skill_config_ambiguous_first");
    let second_runtime_root = make_temp_runtime_root("skill_config_ambiguous_second");
    if first_runtime_root.exists() {
        let _ = fs::remove_dir_all(&first_runtime_root);
    }
    if second_runtime_root.exists() {
        let _ = fs::remove_dir_all(&second_runtime_root);
    }
    fs::create_dir_all(&first_runtime_root).expect("create first ambiguous runtime root");
    fs::create_dir_all(&second_runtime_root).expect("create second ambiguous runtime root");

    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options: LuaRuntimeHostOptions::default(),
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create ambiguous root test engine");

    let error = engine
        .load_from_roots(&[
            crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: first_runtime_root.join("skills"),
            },
            crate::host::options::RuntimeSkillRoot {
                name: "PROJECT".to_string(),
                skills_dir: second_runtime_root.join("skills"),
            },
        ])
        .expect_err("ambiguous runtime roots should require an explicit config file path");
    assert!(
        error
            .to_string()
            .contains("set host_options.skill_config_file_path explicitly"),
        "unexpected ambiguous root error: {error}"
    );

    let _ = fs::remove_dir_all(&first_runtime_root);
    let _ = fs::remove_dir_all(&second_runtime_root);
}

/// Verify lexically equivalent runtime roots do not get misclassified as ambiguous.
/// 验证词法等价的运行时根目录不会被误判为歧义根目录。
#[test]
fn canonical_skill_config_runtime_root_normalizes_equivalent_runtime_roots() {
    let runtime_root = make_temp_runtime_root("skill_config_equivalent_runtime_root");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);

    let engine = LuaEngine::new(LuaEngineOptions {
        host_options: LuaRuntimeHostOptions::default(),
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create equivalent runtime root test engine");

    let equivalent_root = runtime_root.join("nested").join("..").join("skills");
    let resolved_runtime_root = engine
        .canonical_skill_config_runtime_root(&[
            crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("skills"),
            },
            crate::host::options::RuntimeSkillRoot {
                name: "PROJECT".to_string(),
                skills_dir: equivalent_root,
            },
        ])
        .expect("equivalent runtime roots should resolve to one canonical root");

    assert_eq!(resolved_runtime_root, runtime_root);

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify one loaded skill can read its own namespaced config through `vulcan.config.get`.
/// 验证单个已加载技能可以通过 `vulcan.config.get` 读取自己的命名空间配置。
#[test]
fn call_skill_reads_own_skill_config_namespace() {
    let runtime_root = make_temp_runtime_root("skill_config_call_skill");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);
    write_skill_config_test_skill(&runtime_root, "demo-skill");

    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options: LuaRuntimeHostOptions::default(),
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create call_skill config test engine");
    engine
        .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("skills"),
        }])
        .expect("load config test skill");
    engine
        .set_skill_config_value("demo-skill", "api_token", "sk-from-config")
        .expect("seed skill config value");

    let result = engine
        .call_skill("demo-skill-ping", &json!({}), None)
        .expect("call skill with config");
    assert_eq!(result.content, "sk-from-config");

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify `vulcan.config.*` rejects calls that execute without one active skill context.
/// 验证 `vulcan.config.*` 会拒绝在没有活动技能上下文时执行的调用。
#[test]
fn run_lua_config_api_requires_active_skill_context() {
    let engine = make_runtime_test_engine();
    let error = engine
        .run_lua("return vulcan.config.get('api_token')", &json!({}), None)
        .expect_err("run_lua config access should require active skill context");
    assert!(error.contains("vulcan.config.get requires one active skill context"));
}

/// Verify `vulcan.models.*` reports disabled capabilities and structured unavailable errors by default.
/// 验证 `vulcan.models.*` 默认报告能力未开启，并返回结构化不可用错误。
#[test]
fn vulcan_models_defaults_without_callbacks() {
    let _guard = runtime_model_callback_test_guard();
    let engine = make_runtime_test_engine();
    let result = engine
        .run_lua(
            r#"
local status = vulcan.models.status()
local embed = vulcan.models.embed("x")
local llm = vulcan.models.llm("s", "u")
return {
  status_ok = status.ok,
  embed_capability = status.capabilities.embed,
  llm_capability = status.capabilities.llm,
  has_embed = vulcan.models.has("embed"),
  has_llm = vulcan.models.has("llm"),
  has_unknown = vulcan.models.has("rerank"),
  embed_ok = embed.ok,
  embed_code = embed.error.code,
  llm_ok = llm.ok,
  llm_code = llm.error.code,
}
"#,
            &json!({}),
            None,
        )
        .expect("run model defaults lua");

    assert_eq!(result["status_ok"], true);
    assert_eq!(result["embed_capability"], false);
    assert_eq!(result["llm_capability"], false);
    assert_eq!(result["has_embed"], false);
    assert_eq!(result["has_llm"], false);
    assert_eq!(result["has_unknown"], false);
    assert_eq!(result["embed_ok"], false);
    assert_eq!(result["embed_code"], "model_unavailable");
    assert_eq!(result["llm_ok"], false);
    assert_eq!(result["llm_code"], "model_unavailable");
}

/// Verify model APIs return structured invalid-argument errors instead of throwing to Lua.
/// 验证模型 API 会返回结构化非法参数错误，而不是向 Lua 抛出异常。
#[test]
fn vulcan_models_validate_arguments() {
    let _guard = runtime_model_callback_test_guard();
    let engine = make_runtime_test_engine();
    let result = engine
        .run_lua(
            r#"
local embed_empty = vulcan.models.embed("")
local embed_table = vulcan.models.embed({ "a", "b" })
local embed_extra = vulcan.models.embed("x", "extra")
local llm_empty_system = vulcan.models.llm("", "u")
local llm_empty_user = vulcan.models.llm("s", "")
local llm_extra = vulcan.models.llm("s", "u", "extra")
return {
  embed_empty = embed_empty.error.code,
  embed_table = embed_table.error.code,
  embed_extra = embed_extra.error.code,
  llm_empty_system = llm_empty_system.error.code,
  llm_empty_user = llm_empty_user.error.code,
  llm_extra = llm_extra.error.code,
}
"#,
            &json!({}),
            None,
        )
        .expect("run model argument validation lua");

    assert_eq!(result["embed_empty"], "invalid_argument");
    assert_eq!(result["embed_table"], "invalid_argument");
    assert_eq!(result["embed_extra"], "invalid_argument");
    assert_eq!(result["llm_empty_system"], "invalid_argument");
    assert_eq!(result["llm_empty_user"], "invalid_argument");
    assert_eq!(result["llm_extra"], "invalid_argument");
}

/// Verify registered embedding callbacks receive text and full caller context.
/// 验证已注册的 embedding 回调会收到文本和完整调用方上下文。
#[test]
fn vulcan_models_embed_dispatches_registered_callback_with_context() {
    let _guard = runtime_model_callback_test_guard();
    let captured_request: Arc<Mutex<Option<RuntimeModelEmbedRequest>>> = Arc::new(Mutex::new(None));
    let captured_request_for_callback = captured_request.clone();
    set_model_embed_callback(Some(Arc::new(move |request| {
        *captured_request_for_callback
            .lock()
            .expect("lock captured embed request") = Some(request.clone());
        Ok(RuntimeModelEmbedResponse {
            vector: vec![0.25, 0.5, 0.75],
            dimensions: 3,
            usage: Some(RuntimeModelUsage {
                input_tokens: Some(2),
                output_tokens: None,
            }),
        })
    })));

    let engine = make_runtime_test_engine();
    let has_embed = engine
        .run_lua("return vulcan.models.has('embed')", &json!({}), None)
        .expect("run has embed lua");
    assert_eq!(has_embed, json!(true));

    let runtime_root = make_temp_runtime_root("model-embed-context");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);
    let skill_dir = write_model_test_skill_to_root(
        &runtime_root.join("skills"),
        "model-skill",
        "return function(args)\n  local result = vulcan.models.embed(\"hello\")\n  return vulcan.json.encode(result)\nend\n",
    );
    let mut engine = make_runtime_test_engine();
    engine
        .load_from_roots(&[RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("skills"),
        }])
        .expect("load model embed test skill");
    let invocation_context = crate::runtime_options::LuaInvocationContext::new(
        Some(RuntimeRequestContext {
            request_id: Some("req-embed-1".to_string()),
            client_name: Some("Codex Desktop".to_string()),
            transport_name: Some("mcp".to_string()),
            session_id: Some("session-embed".to_string()),
            client_info: Some(RuntimeClientInfo {
                kind: Some("desktop".to_string()),
                name: Some("Codex Desktop".to_string()),
                version: Some("test".to_string()),
            }),
            client_capabilities: json!({"models": true}),
        }),
        json!({"budget": "test"}),
        json!({"tool": "config"}),
    );
    let result = engine
        .call_skill("model-skill-ping", &json!({}), Some(&invocation_context))
        .expect("call model embed skill");
    let result_json: Value =
        serde_json::from_str(&result.content).expect("parse embed result json");
    let captured = captured_request
        .lock()
        .expect("lock captured embed request")
        .clone()
        .expect("embed request captured");

    assert_eq!(result_json["ok"], true);
    assert_eq!(result_json["vector"], json!([0.25, 0.5, 0.75]));
    assert_eq!(result_json["dimensions"], 3);
    assert_eq!(result_json["usage"]["input_tokens"], 2);
    assert_eq!(captured.text, "hello");
    assert_eq!(captured.caller.skill_id.as_deref(), Some("model-skill"));
    assert_eq!(captured.caller.entry_name.as_deref(), Some("ping"));
    assert_eq!(
        captured.caller.canonical_tool_name.as_deref(),
        Some("model-skill-ping")
    );
    assert_eq!(captured.caller.root_name.as_deref(), Some("ROOT"));
    assert_eq!(
        captured.caller.skill_dir.as_deref(),
        Some(skill_dir.to_string_lossy().as_ref())
    );
    assert_eq!(
        captured.caller.client_name.as_deref(),
        Some("Codex Desktop")
    );
    assert_eq!(captured.caller.request_id.as_deref(), Some("req-embed-1"));

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify registered LLM callbacks receive prompts and full caller context.
/// 验证已注册的 LLM 回调会收到提示词和完整调用方上下文。
#[test]
fn vulcan_models_llm_dispatches_registered_callback_with_context() {
    let _guard = runtime_model_callback_test_guard();
    let captured_request: Arc<Mutex<Option<RuntimeModelLlmRequest>>> = Arc::new(Mutex::new(None));
    let captured_request_for_callback = captured_request.clone();
    set_model_llm_callback(Some(Arc::new(move |request| {
        *captured_request_for_callback
            .lock()
            .expect("lock captured llm request") = Some(request.clone());
        Ok(RuntimeModelLlmResponse {
            assistant: "assistant text".to_string(),
            usage: Some(RuntimeModelUsage {
                input_tokens: Some(5),
                output_tokens: Some(7),
            }),
        })
    })));

    let engine = make_runtime_test_engine();
    let has_llm = engine
        .run_lua("return vulcan.models.has('llm')", &json!({}), None)
        .expect("run has llm lua");
    assert_eq!(has_llm, json!(true));

    let runtime_root = make_temp_runtime_root("model-llm-context");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    create_runtime_test_layout(&runtime_root);
    let skill_dir = write_model_test_skill_to_root(
        &runtime_root.join("skills"),
        "llm-skill",
        "return function(args)\n  local result = vulcan.models.llm(\"system\", \"user\")\n  return vulcan.json.encode(result)\nend\n",
    );
    let mut engine = make_runtime_test_engine();
    engine
        .load_from_roots(&[RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("skills"),
        }])
        .expect("load model llm test skill");
    let result = engine
        .call_skill("llm-skill-ping", &json!({}), None)
        .expect("call model llm skill");
    let result_json: Value = serde_json::from_str(&result.content).expect("parse llm result json");
    let captured = captured_request
        .lock()
        .expect("lock captured llm request")
        .clone()
        .expect("llm request captured");

    assert_eq!(result_json["ok"], true);
    assert_eq!(result_json["assistant"], "assistant text");
    assert_eq!(result_json["usage"]["input_tokens"], 5);
    assert_eq!(result_json["usage"]["output_tokens"], 7);
    assert_eq!(captured.system, "system");
    assert_eq!(captured.user, "user");
    assert_eq!(captured.caller.skill_id.as_deref(), Some("llm-skill"));
    assert_eq!(captured.caller.entry_name.as_deref(), Some("ping"));
    assert_eq!(
        captured.caller.canonical_tool_name.as_deref(),
        Some("llm-skill-ping")
    );
    assert_eq!(captured.caller.root_name.as_deref(), Some("ROOT"));
    assert_eq!(
        captured.caller.skill_dir.as_deref(),
        Some(skill_dir.to_string_lossy().as_ref())
    );

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify callback errors preserve standard codes and provider raw fields.
/// 验证回调错误会保留标准错误码和 provider 原始字段。
#[test]
fn vulcan_models_wrap_callback_errors_and_provider_fields() {
    let _guard = runtime_model_callback_test_guard();
    set_model_embed_callback(Some(Arc::new(|_| {
        Err(RuntimeModelError {
            code: RuntimeModelErrorCode::ProviderError,
            message: "provider failed".to_string(),
            provider_message: Some("raw provider message".to_string()),
            provider_code: Some("model_not_found".to_string()),
            provider_status: Some(400),
        })
    })));
    set_model_llm_callback(Some(Arc::new(|_| {
        Err(RuntimeModelError {
            code: RuntimeModelErrorCode::Timeout,
            message: "llm timed out".to_string(),
            provider_message: None,
            provider_code: None,
            provider_status: None,
        })
    })));

    let engine = make_runtime_test_engine();
    let result = engine
        .run_lua(
            r#"
local embed = vulcan.models.embed("hello")
local llm = vulcan.models.llm("system", "user")
return {
  embed_ok = embed.ok,
  embed_code = embed.error.code,
  embed_message = embed.error.message,
  provider_message = embed.error.provider_message,
  provider_code = embed.error.provider_code,
  provider_status = embed.error.provider_status,
  llm_ok = llm.ok,
  llm_code = llm.error.code,
  llm_message = llm.error.message,
}
"#,
            &json!({}),
            None,
        )
        .expect("run model error wrapping lua");

    assert_eq!(result["embed_ok"], false);
    assert_eq!(result["embed_code"], "provider_error");
    assert_eq!(result["embed_message"], "provider failed");
    assert_eq!(result["provider_message"], "raw provider message");
    assert_eq!(result["provider_code"], "model_not_found");
    assert_eq!(result["provider_status"], 400);
    assert_eq!(result["llm_ok"], false);
    assert_eq!(result["llm_code"], "timeout");
    assert_eq!(result["llm_message"], "llm timed out");
}

/// Verify `vulcan.host.*` returns safe defaults when no host callback is registered.
/// 验证未注册宿主回调时 `vulcan.host.*` 会返回安全默认值。
#[test]
fn vulcan_host_bridge_defaults_without_callback() {
    let _guard = host_tool_callback_test_guard();
    let engine = make_runtime_test_engine();
    let result = engine
        .run_lua(
            r#"
local tools = vulcan.host.list()
local called = vulcan.host.call("model.embed", {})
return {
  list_len = #tools,
  has = vulcan.host.has("model.embed"),
  has_tool = vulcan.host.has_tool("model.embed"),
  call_ok = called.ok,
  call_code = called.error.code,
}
"#,
            &json!({}),
            None,
        )
        .expect("run host bridge default lua");

    assert_eq!(result["list_len"], 0);
    assert_eq!(result["has"], false);
    assert_eq!(result["has_tool"], false);
    assert_eq!(result["call_ok"], false);
    assert_eq!(result["call_code"], "host_tool_callback_missing");
}

/// Verify `vulcan.host.*` dispatches list, has, and call requests through the host callback.
/// 验证 `vulcan.host.*` 会通过宿主回调分发 list、has 与 call 请求。
#[test]
fn vulcan_host_bridge_dispatches_registered_callback() {
    let _guard = host_tool_callback_test_guard();
    set_host_tool_callback(Some(Arc::new(|request| match request.action {
        RuntimeHostToolAction::List => Ok(json!([
            {
                "name": "model.echo",
                "description": "Echo test host tool",
                "input_schema": {
                    "type": "object",
                },
            }
        ])),
        RuntimeHostToolAction::Has => Ok(json!(request.tool_name.as_deref() == Some("model.echo"))),
        RuntimeHostToolAction::Call => {
            let tool_name = request.tool_name.as_deref().unwrap_or_default();
            if tool_name != "model.echo" {
                return Err(format!("host tool not found: {}", tool_name));
            }
            Ok(json!({
                "ok": true,
                "value": {
                    "echo": request.args["text"].clone(),
                },
                "meta": {
                    "tool": tool_name,
                },
            }))
        }
    })));

    let engine = make_runtime_test_engine();
    let result = engine
        .run_lua(
            r#"
local tools = vulcan.host.list()
local called = vulcan.host.call("model.echo", { text = "hello" })
return {
  first = tools[1].name,
  has = vulcan.host.has("model.echo"),
  missing = vulcan.host.has_tool("missing.tool"),
  ok = called.ok,
  echo = called.value.echo,
  tool = called.meta.tool,
}
"#,
            &json!({}),
            None,
        )
        .expect("run host bridge callback lua");

    assert_eq!(result["first"], "model.echo");
    assert_eq!(result["has"], true);
    assert_eq!(result["missing"], false);
    assert_eq!(result["ok"], true);
    assert_eq!(result["echo"], "hello");
    assert_eq!(result["tool"], "model.echo");
}

/// Verify `vulcan.host.call` converts callback failures into table error envelopes.
/// 验证 `vulcan.host.call` 会把回调失败转换为 table 错误包络。
#[test]
fn vulcan_host_call_wraps_callback_errors() {
    let _guard = host_tool_callback_test_guard();
    set_host_tool_callback(Some(Arc::new(|request| match request.action {
        RuntimeHostToolAction::List => Ok(json!([])),
        RuntimeHostToolAction::Has => Ok(json!(true)),
        RuntimeHostToolAction::Call => {
            assert!(request.args.as_object().is_some());
            assert!(request.args.as_object().unwrap().is_empty());
            Err("model provider failed".to_string())
        }
    })));

    let engine = make_runtime_test_engine();
    let result = engine
        .run_lua(
            r#"
local called = vulcan.host.call("model.fail", {})
return {
  ok = called.ok,
  code = called.error.code,
  message = called.error.message,
}
"#,
            &json!({}),
            None,
        )
        .expect("run host bridge callback error lua");

    assert_eq!(result["ok"], false);
    assert_eq!(result["code"], "host_tool_callback_error");
    assert_eq!(result["message"], "model provider failed");
}

/// Assert that one pooled Lua VM has returned to the neutral request baseline.
/// 断言单个池化 Lua 虚拟机已经回到中性的请求基线状态。
fn assert_vm_scope_is_clean(lua: &mlua::Lua) {
    let context = get_vulcan_context_table(lua).expect("get vulcan.context");
    let request: Table = context.get("request").expect("get request table");
    assert_eq!(request.raw_len(), 0);
    assert_eq!(request.pairs::<String, LuaValue>().count(), 0);
    assert!(matches!(
        context
            .get::<LuaValue>("client_info")
            .expect("get client_info"),
        LuaValue::Nil
    ));
    assert!(matches!(
        context
            .get::<LuaValue>("client_capabilities")
            .expect("get client_capabilities"),
        LuaValue::Table(_)
    ));
    assert!(matches!(
        context
            .get::<LuaValue>("client_budget")
            .expect("get client_budget"),
        LuaValue::Table(_)
    ));
    assert!(matches!(
        context
            .get::<LuaValue>("tool_config")
            .expect("get tool_config"),
        LuaValue::Table(_)
    ));
    assert!(matches!(
        context.get::<LuaValue>("skill_dir").expect("get skill_dir"),
        LuaValue::Nil
    ));
    assert!(matches!(
        context.get::<LuaValue>("entry_dir").expect("get entry_dir"),
        LuaValue::Nil
    ));
    assert!(matches!(
        context
            .get::<LuaValue>("entry_file")
            .expect("get entry_file"),
        LuaValue::Nil
    ));

    let deps = get_vulcan_deps_table(lua).expect("get vulcan.deps");
    assert!(matches!(
        deps.get::<LuaValue>("tools_path").expect("get tools_path"),
        LuaValue::Nil
    ));
    assert!(matches!(
        deps.get::<LuaValue>("lua_path").expect("get lua_path"),
        LuaValue::Nil
    ));
    assert!(matches!(
        deps.get::<LuaValue>("ffi_path").expect("get ffi_path"),
        LuaValue::Nil
    ));

    let internal = get_vulcan_runtime_internal_table(lua).expect("get runtime internal");
    assert!(matches!(
        internal
            .get::<LuaValue>("tool_name")
            .expect("get tool_name"),
        LuaValue::Nil
    ));
    assert!(matches!(
        internal
            .get::<LuaValue>("skill_name")
            .expect("get skill_name"),
        LuaValue::Nil
    ));
    assert!(matches!(
        internal
            .get::<LuaValue>("entry_name")
            .expect("get entry_name"),
        LuaValue::Nil
    ));
    assert!(matches!(
        internal
            .get::<LuaValue>("root_name")
            .expect("get root_name"),
        LuaValue::Nil
    ));
    assert!(
        !internal
            .get::<bool>("luaexec_active")
            .expect("get luaexec_active")
    );
    assert!(matches!(
        internal
            .get::<LuaValue>("luaexec_caller_tool_name")
            .expect("get luaexec_caller_tool_name"),
        LuaValue::Nil
    ));

    let vulcan = get_vulcan_table(lua).expect("get vulcan");
    let lancedb: Table = vulcan.get("lancedb").expect("get lancedb");
    assert!(!lancedb.get::<bool>("enabled").expect("get lancedb enabled"));
    let sqlite: Table = vulcan.get("sqlite").expect("get sqlite");
    assert!(!sqlite.get::<bool>("enabled").expect("get sqlite enabled"));
    assert!(matches!(
        lua.globals()
            .get::<LuaValue>("__runlua_args")
            .expect("get __runlua_args"),
        LuaValue::Nil
    ));
}

/// Verify that skill manifests must not declare skill_id explicitly.
/// 验证 skill 清单不允许再显式声明 skill_id 字段。
#[test]
fn load_from_roots_rejects_explicit_skill_id_field() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_reject_skill_id_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = fs::remove_dir_all(&temp_root);
    }
    let skill_root = temp_root.join("skills");
    let skill_dir = skill_root.join("vulcan-codekit");
    fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime dir");
    fs::write(
            skill_dir.join("skill.yaml"),
            "name: vulcan-codekit\nversion: 0.1.0\nskill_id: vulcan-codekit\nentries:\n  - name: ast-tree\n    lua_entry: runtime/test.lua\n    lua_module: vulcan-codekit.ast-tree\n",
        )
        .expect("write skill yaml");
    fs::write(skill_dir.join("runtime").join("test.lua"), "return 'ok'\n")
        .expect("write runtime entry");

    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options: LuaRuntimeHostOptions::default(),
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create engine");

    let error = engine
        .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: skill_root,
        }])
        .expect_err("explicit skill_id should be rejected");
    let rendered = error.to_string();
    assert!(rendered.contains("must not declare skill_id"));

    let _ = fs::remove_dir_all(&temp_root);
}

/// Verify that host-ignored skills are skipped before dependency, database, or entry setup.
/// 验证宿主忽略的 skill 会在依赖、数据库与入口初始化之前被跳过。
#[test]
fn load_from_roots_skips_host_ignored_skill_before_resource_setup() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_ignored_skill_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = fs::remove_dir_all(&temp_root);
    }
    let skill_root = temp_root.join("skills");
    let skill_dir = skill_root.join("grpc-memory");
    fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime dir");
    fs::write(
            skill_dir.join("skill.yaml"),
            "name: grpc-memory\nversion: 0.1.0\nenable: true\ndebug: false\nsqlite:\n  enable: true\nlancedb:\n  enable: true\nentries:\n  - name: remember\n    lua_entry: runtime/remember.lua\n    lua_module: grpc-memory.remember\n",
        )
        .expect("write skill yaml");
    fs::write(
        skill_dir.join("runtime").join("remember.lua"),
        "return function(args)\n  return 'unexpected-load'\nend\n",
    )
    .expect("write runtime entry");

    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.dependency_dir_name = "dependencies".to_string();
    host_options.state_dir_name = "state".to_string();
    host_options.database_dir_name = "databases".to_string();
    host_options.ignored_skill_ids = vec!["grpc-memory".to_string()];
    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options,
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create engine");

    engine
        .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: skill_root,
        }])
        .expect("ignored skill should not fail loading");

    assert!(engine.skills.is_empty());
    assert!(engine.entry_registry.is_empty());
    assert!(!temp_root.join("dependencies").exists());
    assert!(!temp_root.join("state").exists());
    assert!(!temp_root.join("databases").exists());

    let _ = fs::remove_dir_all(&temp_root);
}

/// Verify that colliding `skill-entry` names receive deterministic numeric suffixes.
/// 验证发生冲突的 `skill-entry` 名称会收到稳定且可预测的数字后缀。
#[test]
fn rebuild_entry_registry_appends_numeric_suffixes_for_collisions() {
    let mut skills = HashMap::new();
    skills.insert(
        "alpha".to_string(),
        make_loaded_skill("alpha", "foo-bar", "baz", "alpha_module"),
    );
    skills.insert(
        "beta".to_string(),
        make_loaded_skill("beta", "foo", "bar-baz", "beta_module"),
    );
    skills.insert(
        "gamma".to_string(),
        make_loaded_skill("gamma", "foo-bar", "baz", "gamma_module"),
    );

    let mut engine = make_test_engine(skills);
    engine
        .rebuild_entry_registry()
        .expect("entry registry should rebuild successfully");

    assert!(engine.entry_registry.contains_key("foo-bar-baz"));
    assert!(engine.entry_registry.contains_key("foo-bar-baz-2"));
    assert!(engine.entry_registry.contains_key("foo-bar-baz-3"));

    let alpha_skill = engine
        .skills
        .get("alpha")
        .expect("alpha skill should exist");
    let beta_skill = engine.skills.get("beta").expect("beta skill should exist");
    let gamma_skill = engine
        .skills
        .get("gamma")
        .expect("gamma skill should exist");

    assert_eq!(alpha_skill.resolved_tool_name("baz"), Some("foo-bar-baz"));
    assert_eq!(
        beta_skill.resolved_tool_name("bar-baz"),
        Some("foo-bar-baz-2")
    );
    assert_eq!(gamma_skill.resolved_tool_name("baz"), Some("foo-bar-baz-3"));
}

/// Verify that host-reserved public tool names are treated as occupied during canonical-name generation.
/// 验证宿主保留的公开工具名称会在 canonical 名称生成阶段被视为已占用名称。
#[test]
fn rebuild_entry_registry_skips_host_reserved_names() {
    let mut skills = HashMap::new();
    skills.insert(
        "alpha".to_string(),
        make_loaded_skill("alpha", "vulcan", "help-list", "alpha_module"),
    );

    let mut engine = make_test_engine(skills);
    Arc::get_mut(&mut engine.host_options)
        .expect("host options should be uniquely owned in test")
        .reserved_entry_names = vec!["vulcan-help-list".to_string()];

    engine
        .rebuild_entry_registry()
        .expect("entry registry should rebuild successfully");

    assert!(!engine.entry_registry.contains_key("vulcan-help-list"));
    assert!(engine.entry_registry.contains_key("vulcan-help-list-2"));

    let alpha_skill = engine
        .skills
        .get("alpha")
        .expect("alpha skill should exist");
    assert_eq!(
        alpha_skill.resolved_tool_name("help-list"),
        Some("vulcan-help-list-2")
    );
}

/// Verify that the pooled VM scope guard clears request state even when setup exits early.
/// 验证池化虚拟机作用域守卫即使在安装阶段提前退出也会清理请求状态。
#[test]
fn pooled_vm_scope_guard_cleans_state_after_early_exit() {
    let engine = make_runtime_test_engine();
    let scope_result: Result<(), String> = (|| {
        let mut lease = engine.acquire_vm()?;
        let _scope_guard = LuaVmRequestScopeGuard::new(&mut lease, engine.host_options.as_ref())?;
        let lua = _scope_guard.lua();
        LuaEngine::populate_vulcan_request_context(
            lua,
            Some(&crate::runtime_options::LuaInvocationContext::new(
                None,
                json!({"budget":"test"}),
                json!({"tool":"config"}),
            )),
        )?;
        populate_vulcan_internal_execution_context(
            lua,
            &VulcanInternalExecutionContext {
                tool_name: Some("test-tool".to_string()),
                skill_name: Some("test-skill".to_string()),
                entry_name: Some("test".to_string()),
                root_name: Some("ROOT".to_string()),
                luaexec_active: false,
                luaexec_caller_tool_name: None,
            },
        )?;
        let skill_dir = Path::new("D:/runtime-test-root/skills/test-skill");
        let entry_file = Path::new("D:/runtime-test-root/skills/test-skill/runtime/test.lua");
        populate_vulcan_file_context(lua, Some(skill_dir), Some(entry_file))?;
        populate_vulcan_dependency_context(
            lua,
            engine.host_options.as_ref(),
            Some(skill_dir),
            Some("test-skill"),
        )?;
        lua.globals()
            .set(
                "__runlua_args",
                json_to_lua_table(lua, &json!({"stale":"value"})).expect("build runlua args table"),
            )
            .expect("set stale runlua args");
        Err("simulated setup failure".to_string())
    })();
    assert_eq!(
        scope_result.expect_err("scope should fail"),
        "simulated setup failure"
    );

    let lease = engine.acquire_vm().expect("reacquire pooled vm");
    assert_vm_scope_is_clean(lease.lua());
}

/// Verify that a pooled VM with broken core tables is discarded before it can be reused.
/// 验证当池化虚拟机的核心表被破坏时，该实例会在复用前被直接丢弃。
#[test]
fn pooled_vm_scope_guard_discards_vm_when_entry_reset_fails() {
    let engine = make_runtime_test_engine();
    {
        let lease = engine.acquire_vm().expect("borrow pooled vm");
        let vulcan = get_vulcan_table(lease.lua()).expect("get vulcan");
        vulcan
            .set("context", LuaValue::Nil)
            .expect("break vulcan.context");
    }

    let mut broken_lease = engine.acquire_vm().expect("reacquire broken pooled vm");
    let error = match LuaVmRequestScopeGuard::new(&mut broken_lease, engine.host_options.as_ref()) {
        Ok(_) => panic!("broken pooled vm should fail normalization"),
        Err(error) => error,
    };
    assert!(error.contains("vulcan.context"));

    let mut fresh_lease = engine.acquire_vm().expect("borrow fresh pooled vm");
    let fresh_scope = LuaVmRequestScopeGuard::new(&mut fresh_lease, engine.host_options.as_ref())
        .expect("normalize fresh pooled vm");
    assert_vm_scope_is_clean(fresh_scope.lua());
}

/// Verify that cleanup failures retire the current pooled VM instead of returning dirty state.
/// 验证当清理阶段失败时，当前池化虚拟机会被退役，而不是带着脏状态返回池中。
#[test]
fn pooled_vm_scope_guard_discards_vm_when_exit_reset_fails() {
    let engine = make_runtime_test_engine();
    let mut lease = engine.acquire_vm().expect("borrow pooled vm");
    let scope_guard = LuaVmRequestScopeGuard::new(&mut lease, engine.host_options.as_ref())
        .expect("normalize pooled vm");
    let vulcan = get_vulcan_table(scope_guard.lua()).expect("get vulcan");
    vulcan
        .set("context", LuaValue::Nil)
        .expect("break vulcan.context");
    let error = scope_guard
        .finish()
        .expect_err("cleanup should fail after context corruption");
    assert!(error.contains("vulcan.context"));

    let mut fresh_lease = engine.acquire_vm().expect("borrow fresh pooled vm");
    let fresh_scope = LuaVmRequestScopeGuard::new(&mut fresh_lease, engine.host_options.as_ref())
        .expect("normalize fresh pooled vm");
    assert_vm_scope_is_clean(fresh_scope.lua());
}

/// Verify that run_lua clears transient args after one successful execution.
/// 验证 run_lua 在成功执行后会清理临时参数状态。
#[test]
fn run_lua_clears_args_after_success() {
    let engine = make_runtime_test_engine();
    let result = engine
        .run_lua("return args.value", &json!({"value":"hello"}), None)
        .expect("run_lua should succeed");
    assert_eq!(result, json!("hello"));

    let lease = engine.acquire_vm().expect("reacquire pooled vm");
    assert_vm_scope_is_clean(lease.lua());
}

/// Verify isolated `vulcan.runtime.lua.exec` calls reuse the dedicated runlua VM pool.
/// 验证隔离 `vulcan.runtime.lua.exec` 调用会复用独立的 runlua 虚拟机池。
#[test]
fn execute_runlua_request_inline_reuses_dedicated_pool() {
    let engine = make_runtime_test_engine();
    assert_eq!(engine.runlua_pool.total_count(), 0);

    let first = engine
        .execute_runlua_request_json_inline(r#"{"code":"return 1"}"#)
        .expect("first inline runlua should succeed");
    assert!(!first.trim().is_empty());
    assert_eq!(engine.runlua_pool.total_count(), 1);

    let second = engine
        .execute_runlua_request_json_inline(r#"{"code":"return 2"}"#)
        .expect("second inline runlua should succeed");
    assert!(!second.trim().is_empty());
    assert_eq!(engine.runlua_pool.total_count(), 1);
}

/// Verify isolated runlua redirects Lua `io.open` to the Rust-backed managed IO table.
/// 验证隔离 runlua 会把 Lua `io.open` 重定向到 Rust 托管 IO 表。
#[test]
fn execute_runlua_request_inline_uses_managed_io_open() {
    let engine = make_runtime_test_engine();
    let path = std::env::temp_dir().join(format!(
        "luaskills_runlua_managed_io_{}.txt",
        std::process::id()
    ));
    fs::write(&path, "managed-io-ok").expect("write managed io test file");
    let request = json!({
        "code": "local f = io.open(args.path, 'r'); local value = f:read('*a'); f:close(); return value",
        "args": {
            "path": path.to_string_lossy().to_string()
        }
    });

    let result = engine
        .execute_runlua_request_json_inline(&request.to_string())
        .expect("inline runlua should read through managed io");

    assert!(result.contains("SUCCESS"));
    assert!(result.contains("managed-io-ok"));
    let _ = fs::remove_file(path);
}

/// Verify isolated runlua supports default managed `io.input` and `io.read`.
/// 验证隔离 runlua 支持托管默认 `io.input` 与 `io.read`。
#[test]
fn execute_runlua_request_inline_uses_managed_io_default_input() {
    let engine = make_runtime_test_engine();
    let path = std::env::temp_dir().join(format!(
        "luaskills_runlua_managed_io_input_{}.txt",
        std::process::id()
    ));
    fs::write(&path, "managed-default-input").expect("write managed input test file");
    let request = json!({
        "code": "io.input(args.path); return io.read('*a')",
        "args": {
            "path": path.to_string_lossy().to_string()
        }
    });

    let result = engine
        .execute_runlua_request_json_inline(&request.to_string())
        .expect("inline runlua should read through managed default input");

    assert!(result.contains("SUCCESS"));
    assert!(result.contains("managed-default-input"));
    let _ = fs::remove_file(path);
}

/// Verify isolated runlua supports default managed `io.output` and `io.write`.
/// 验证隔离 runlua 支持托管默认 `io.output` 与 `io.write`。
#[test]
fn execute_runlua_request_inline_uses_managed_io_default_output() {
    let engine = make_runtime_test_engine();
    let path = std::env::temp_dir().join(format!(
        "luaskills_runlua_managed_io_output_{}.txt",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let request = json!({
        "code": "io.output(args.path); io.write('managed', '-', 'default-output'); io.close(); return vulcan.io.read_text(args.path, { encoding = 'utf-8' })",
        "args": {
            "path": path.to_string_lossy().to_string()
        }
    });

    let result = engine
        .execute_runlua_request_json_inline(&request.to_string())
        .expect("inline runlua should write through managed default output");

    assert!(result.contains("SUCCESS"));
    assert!(result.contains("managed-default-output"));
    let _ = fs::remove_file(path);
}

/// Verify isolated runlua redirects Lua `io.popen` to the Rust-backed read implementation.
/// 验证隔离 runlua 会把 Lua `io.popen` 重定向到 Rust 托管读取实现。
#[test]
fn execute_runlua_request_inline_uses_managed_io_popen() {
    let engine = make_runtime_test_engine();
    let result = engine
            .execute_runlua_request_json_inline(
                r#"{"code":"local f = io.popen('echo managed-popen-ok', 'r'); local value = f:read('*a'); local ok = f:close(); return value, ok"}"#,
            )
            .expect("inline runlua should read through managed io.popen");

    assert!(result.contains("SUCCESS"));
    assert!(result.contains("managed-popen-ok"));
    assert!(result.contains("true"));
}

/// Verify isolated runlua rejects the unsupported managed `io.popen` write mode.
/// 验证隔离 runlua 会拒绝暂不支持的托管 `io.popen` 写入模式。
#[test]
fn execute_runlua_request_inline_rejects_io_popen_write_mode() {
    let engine = make_runtime_test_engine();
    let result = engine
        .execute_runlua_request_json_inline(r#"{"code":"return io.popen('echo hello', 'w')"}"#)
        .expect("inline runlua should render the managed io.popen mode error");

    assert!(result.contains("FAILED"));
    assert!(result.contains("write mode is not implemented yet"));
}

/// Verify host default text encoding is used by managed IO when Lua omits encoding options.
/// 验证 Lua 省略编码选项时托管 IO 会使用宿主默认文本编码。
#[test]
fn execute_runlua_request_inline_uses_host_default_text_encoding() {
    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.default_text_encoding = Some("gb18030".to_string());
    let engine = make_runtime_test_engine_with_host_options(host_options);
    let path = std::env::temp_dir().join(format!(
        "luaskills_runlua_default_encoding_{}.txt",
        std::process::id()
    ));
    let bytes = encode_runtime_text("宿主默认编码", RuntimeTextEncoding::Gb18030)
        .expect("encode host default gb18030 test file");
    fs::write(&path, bytes).expect("write host default encoding file");
    let request = json!({
        "code": "return vulcan.io.read_text(args.path)",
        "args": {
            "path": path.to_string_lossy().to_string()
        }
    });

    let result = engine
        .execute_runlua_request_json_inline(&request.to_string())
        .expect("inline runlua should read through host default encoding");

    assert!(result.contains("SUCCESS"));
    assert!(result.contains("宿主默认编码"));
    let _ = fs::remove_file(path);
}

/// Verify hosts can disable the managed global `io` compatibility layer for luaexec.
/// 验证宿主可以为 luaexec 关闭托管全局 `io` 兼容层。
#[test]
fn execute_runlua_request_inline_can_disable_managed_io_compat() {
    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.capabilities.enable_managed_io_compat = false;
    let engine = make_runtime_test_engine_with_host_options(host_options);
    let result = engine
            .execute_runlua_request_json_inline(
                r#"{"code":"local preload = package and package.preload and package.preload.io; return type(preload) == 'function' and 'managed' or 'native'"}"#,
            )
            .expect("inline runlua should keep native io when managed compat is disabled");

    assert!(result.contains("SUCCESS"));
    assert!(result.contains("native"));
}

/// Verify `vulcan.process.exec` exposes explicit encoding metadata after byte-based capture.
/// 验证 `vulcan.process.exec` 在按字节捕获后会暴露明确的编码元数据。
#[test]
fn execute_runlua_request_inline_reports_process_exec_encoding_metadata() {
    let engine = make_runtime_test_engine();
    let result = engine
            .execute_runlua_request_json_inline(
                r#"{"code":"local info = vulcan.os.info(); local spec; if info.os == 'windows' then spec = { program = 'cmd', args = { '/C', 'echo exec-encoding-ok' }, encoding = 'utf-8' } else spec = { program = 'sh', args = { '-c', 'printf exec-encoding-ok' }, encoding = 'utf-8' } end; local result = vulcan.process.exec(spec); return result.stdout, result.stdout_encoding, result.stdout_lossy"}"#,
            )
            .expect("inline runlua should execute process.exec");

    assert!(result.contains("SUCCESS"));
    assert!(result.contains("exec-encoding-ok"));
    assert!(result.contains("utf-8"));
    assert!(result.contains("false"));
}

/// Verify `vulcan.process.session` can write to stdin and read captured stdout.
/// 验证 `vulcan.process.session` 可以写入 stdin 并读取捕获的 stdout。
#[test]
fn execute_runlua_request_inline_uses_process_session_write_read() {
    let engine = make_runtime_test_engine();
    let result = engine
            .execute_runlua_request_json_inline(
                r#"{"code":"local info = vulcan.os.info(); local spec; if info.os == 'windows' then spec = { program = 'cmd', args = { '/V:ON', '/C', 'set /P line=&echo session:!line!' }, encoding = 'utf-8' } else spec = { program = 'sh', args = { '-c', 'read line; echo session:$line' }, encoding = 'utf-8' } end; local session = vulcan.process.session.open(spec); session:write('ok\\n'); local status = session:close({ timeout_ms = 3000 }); local output = session:read({ timeout_ms = 3000 }); return output.stdout, status.exited, status.success"}"#,
            )
            .expect("inline runlua should exercise process session");

    assert!(result.contains("SUCCESS"));
    assert!(result.contains("session:ok"));
    assert!(result.contains("true"));
}

/// Verify persistent runtime sessions keep Lua VM globals across eval calls.
/// 验证持久运行时会话会在多次 eval 调用之间保留 Lua VM 全局状态。
#[test]
fn runtime_session_eval_preserves_vm_state_across_calls() {
    let engine = make_runtime_test_engine();
    let created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"stateful-test","ttl_sec":60}"#)
            .expect("create runtime session"),
    )
    .expect("create response json");
    assert_eq!(created["ok"], true);
    let lease_id = created["lease_id"]
        .as_str()
        .expect("lease id should be present")
        .to_string();

    let first_request = json!({
        "lease_id": lease_id,
        "code": "counter = (counter or 0) + 1; return counter"
    });
    let first: Value = serde_json::from_str(
        &engine
            .eval_runtime_lease_json(&first_request.to_string())
            .expect("first runtime session eval"),
    )
    .expect("first eval response json");
    assert_eq!(first["ok"], true);
    assert_eq!(first["result"], json!(1));

    let second_request = json!({
        "lease_id": lease_id,
        "code": "counter = (counter or 0) + 1; return counter"
    });
    let second: Value = serde_json::from_str(
        &engine
            .eval_runtime_lease_json(&second_request.to_string())
            .expect("second runtime session eval"),
    )
    .expect("second eval response json");
    assert_eq!(second["ok"], true);
    assert_eq!(second["result"], json!(2));
}

/// Verify system runtime leases preserve one explicit host-owned cwd while still exposing the fixed system_lua_lib directory.
/// 验证 system 运行时租约会保留宿主显式传入的 cwd，同时继续暴露固定的 system_lua_lib 目录。
#[test]
fn system_runtime_lease_preserves_explicit_cwd_override() {
    let runtime_root = make_temp_runtime_root("system-runtime-lease-cwd");
    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root);
    }
    let explicit_cwd = runtime_root.join("host-cwd");
    let fixed_system_dir = runtime_root.join("fixed-system-lua-lib");
    fs::create_dir_all(&explicit_cwd).expect("create explicit host cwd");

    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.system_lua_lib_dir = Some(fixed_system_dir.clone());
    let engine = make_runtime_test_engine_with_host_options(host_options);

    let created: Value = serde_json::from_str(
        &engine
            .create_system_runtime_lease_json(
                &json!({
                    "authority": "system",
                    "sid": "system-cwd-test",
                    "ttl_sec": 60,
                    "cwd": explicit_cwd.to_string_lossy()
                })
                .to_string(),
            )
            .expect("create system runtime lease"),
    )
    .expect("system runtime lease create response json");
    assert_eq!(created["ok"], true);
    assert_eq!(
        created["cwd"],
        json!(render_host_visible_path(&explicit_cwd))
    );
    assert_eq!(
        created["system_lua_lib"],
        json!(render_host_visible_path(&fixed_system_dir))
    );

    let lease_id = created["lease_id"]
        .as_str()
        .expect("lease id should be present")
        .to_string();
    let generation = created["generation"]
        .as_u64()
        .expect("generation should be present");

    let status: Value = serde_json::from_str(
        &engine
            .system_runtime_lease_status_json(
                &json!({
                    "authority": "system",
                    "lease_id": lease_id,
                    "generation": generation
                })
                .to_string(),
            )
            .expect("status system runtime lease"),
    )
    .expect("system runtime lease status response json");
    assert_eq!(status["ok"], true);
    assert_eq!(
        status["cwd"],
        json!(render_host_visible_path(&explicit_cwd))
    );
    assert_eq!(
        status["system_lua_lib"],
        json!(render_host_visible_path(&fixed_system_dir))
    );

    let eval: Value = serde_json::from_str(
        &engine
            .eval_system_runtime_lease_json(
                &json!({
                    "authority": "system",
                    "lease_id": lease_id,
                    "generation": generation,
                    "code": "return { cwd = vulcan.runtime.cwd() }"
                })
                .to_string(),
            )
            .expect("eval system runtime lease"),
    )
    .expect("system runtime lease eval response json");
    assert_eq!(eval["ok"], true);
    assert_eq!(eval["cwd"], json!(render_host_visible_path(&explicit_cwd)));
    assert_eq!(
        eval["system_lua_lib"],
        json!(render_host_visible_path(&fixed_system_dir))
    );
    assert_eq!(
        eval["result"]["cwd"],
        json!(render_host_visible_path(&explicit_cwd))
    );

    let _ = fs::remove_dir_all(&runtime_root);
}

/// Verify closed runtime sessions return a stable lease_closed error.
/// 验证已关闭的运行时会话会返回稳定的 lease_closed 错误。
#[test]
fn runtime_session_eval_reports_closed_lease() {
    let engine = make_runtime_test_engine();
    let created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"closed-test","ttl_sec":60}"#)
            .expect("create runtime session"),
    )
    .expect("create response json");
    let lease_id = created["lease_id"]
        .as_str()
        .expect("lease id should be present")
        .to_string();
    let close_request = json!({ "lease_id": lease_id });
    let closed: Value = serde_json::from_str(
        &engine
            .close_runtime_lease_json(&close_request.to_string())
            .expect("close runtime session"),
    )
    .expect("close response json");
    assert_eq!(closed["ok"], true);
    assert_eq!(closed["closed"], true);

    let eval_request = json!({
        "lease_id": lease_id,
        "code": "return 1"
    });
    let eval: Value = serde_json::from_str(
        &engine
            .eval_runtime_lease_json(&eval_request.to_string())
            .expect("eval closed runtime session"),
    )
    .expect("eval response json");
    assert_eq!(eval["ok"], false);
    assert_eq!(eval["error_code"], "lease_closed");
}

/// Verify closed runtime sessions return a stable lease_closed error from status.
/// 验证已关闭的运行时会话在 status 中会返回稳定的 lease_closed 错误。
#[test]
fn runtime_session_status_reports_closed_lease() {
    let engine = make_runtime_test_engine();
    let created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"closed-status-test","ttl_sec":60}"#)
            .expect("create runtime session"),
    )
    .expect("create response json");
    let lease_id = created["lease_id"]
        .as_str()
        .expect("lease id should be present")
        .to_string();
    let close_request = json!({ "lease_id": lease_id.clone() });
    let closed: Value = serde_json::from_str(
        &engine
            .close_runtime_lease_json(&close_request.to_string())
            .expect("close runtime session"),
    )
    .expect("close response json");
    assert_eq!(closed["ok"], true);

    let status_request = json!({ "lease_id": lease_id });
    let status: Value = serde_json::from_str(
        &engine
            .runtime_lease_status_json(&status_request.to_string())
            .expect("status closed runtime session"),
    )
    .expect("status response json");
    assert_eq!(status["ok"], false);
    assert_eq!(status["error_code"], "lease_closed");
}

/// Verify replaced runtime sessions keep a stable lease_replaced terminal error.
/// 验证被替换的运行时会话会保留稳定的 lease_replaced 终态错误。
#[test]
fn runtime_session_eval_reports_replaced_lease() {
    let engine = make_runtime_test_engine();
    let first_created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"replace-test","ttl_sec":60}"#)
            .expect("create first runtime session"),
    )
    .expect("first create response json");
    let first_lease_id = first_created["lease_id"]
        .as_str()
        .expect("first lease id should be present")
        .to_string();

    let second_created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"replace-test","ttl_sec":60,"replace":true}"#)
            .expect("create second runtime session"),
    )
    .expect("second create response json");
    assert_eq!(second_created["ok"], true);
    assert_ne!(second_created["lease_id"], first_created["lease_id"]);

    let eval_request = json!({
        "lease_id": first_lease_id,
        "code": "return 1"
    });
    let eval: Value = serde_json::from_str(
        &engine
            .eval_runtime_lease_json(&eval_request.to_string())
            .expect("eval replaced runtime session"),
    )
    .expect("replaced eval response json");
    assert_eq!(eval["ok"], false);
    assert_eq!(eval["error_code"], "lease_replaced");
}

/// Verify replaced runtime sessions return a stable lease_replaced error from status.
/// 验证被替换的运行时会话在 status 中会返回稳定的 lease_replaced 错误。
#[test]
fn runtime_session_status_reports_replaced_lease() {
    let engine = make_runtime_test_engine();
    let first_created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"replace-status-test","ttl_sec":60}"#)
            .expect("create first runtime session"),
    )
    .expect("first create response json");
    let first_lease_id = first_created["lease_id"]
        .as_str()
        .expect("first lease id should be present")
        .to_string();

    let second_created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(
                r#"{"sid":"replace-status-test","ttl_sec":60,"replace":true}"#,
            )
            .expect("create second runtime session"),
    )
    .expect("second create response json");
    assert_eq!(second_created["ok"], true);

    let status_request = json!({ "lease_id": first_lease_id });
    let status: Value = serde_json::from_str(
        &engine
            .runtime_lease_status_json(&status_request.to_string())
            .expect("status replaced runtime session"),
    )
    .expect("status response json");
    assert_eq!(status["ok"], false);
    assert_eq!(status["error_code"], "lease_replaced");
}

/// Verify a stale runtime-session handle observes lease_replaced after another caller replaces the SID lease.
/// 验证陈旧运行时会话句柄会在另一个调用方替换同 SID 租约后观察到 lease_replaced。
#[test]
fn runtime_session_stale_handle_reports_replaced_after_manager_get() {
    let engine = make_runtime_test_engine();
    let first_created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"replace-race-test","ttl_sec":60}"#)
            .expect("create first runtime session"),
    )
    .expect("first create response json");
    let first_lease_id = first_created["lease_id"]
        .as_str()
        .expect("first lease id should be present")
        .to_string();
    let stale_session = engine
        .runtime_sessions
        .get(&first_lease_id, None, None, None)
        .expect("capture stale runtime session handle");

    let replaced: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"replace-race-test","ttl_sec":60,"replace":true}"#)
            .expect("replace runtime session"),
    )
    .expect("replace response json");
    assert_eq!(replaced["ok"], true);

    let mut stale_session = stale_session.lock().expect("lock stale runtime session");
    let error = LuaEngine::ensure_runtime_session_active(&mut stale_session)
        .expect_err("stale handle should fail");
    assert_eq!(error.code, "lease_replaced");
}

/// Verify replace=true rejects one busy lease before creating a second VM for the same SID.
/// 验证 replace=true 会在同一 SID 的旧租约忙碌时拒绝替换，而不会创建第二个虚拟机。
#[test]
fn runtime_session_replace_rejects_busy_lease() {
    let engine = make_runtime_test_engine();
    let created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"busy-replace-test","ttl_sec":60}"#)
            .expect("create busy replace runtime session"),
    )
    .expect("busy replace create response json");
    let lease_id = created["lease_id"]
        .as_str()
        .expect("busy replace lease id should be present")
        .to_string();

    let session = engine
        .runtime_sessions
        .get(&lease_id, None, None, None)
        .expect("get busy replace runtime session");
    let guard = session.lock().expect("lock busy replace runtime session");

    let blocked_replace: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"busy-replace-test","ttl_sec":60,"replace":true}"#)
            .expect("replace busy runtime session"),
    )
    .expect("busy replace response json");
    assert_eq!(blocked_replace["ok"], false);
    assert_eq!(blocked_replace["error_code"], "lease_busy");
    assert!(
        blocked_replace["message"]
            .as_str()
            .expect("busy replace message should be present")
            .contains("cannot replace busy lease")
    );

    let listed: Value = serde_json::from_str(
        &engine
            .list_runtime_leases_json(r#"{"sid":"busy-replace-test"}"#)
            .expect("list busy replace runtime sessions"),
    )
    .expect("busy replace list response json");
    assert_eq!(listed["ok"], true);
    assert_eq!(listed["leases"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed["leases"][0]["lease_id"], lease_id);

    drop(guard);

    let replaced: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"busy-replace-test","ttl_sec":60,"replace":true}"#)
            .expect("replace idle runtime session"),
    )
    .expect("idle replace response json");
    assert_eq!(replaced["ok"], true);
    assert_ne!(replaced["lease_id"], created["lease_id"]);
}

/// Verify runtime sessions reject a mismatched echoed SID before executing code.
/// 验证运行时会话会在执行前拒绝不匹配的回传 SID。
#[test]
fn runtime_session_eval_rejects_sid_mismatch() {
    let engine = make_runtime_test_engine();
    let created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"identity-test","ttl_sec":60}"#)
            .expect("create identity runtime session"),
    )
    .expect("identity create response json");
    let lease_id = created["lease_id"]
        .as_str()
        .expect("identity lease id should be present")
        .to_string();

    let eval_request = json!({
        "lease_id": lease_id,
        "sid": "wrong-sid",
        "code": "return 1"
    });
    let eval: Value = serde_json::from_str(
        &engine
            .eval_runtime_lease_json(&eval_request.to_string())
            .expect("eval runtime session with wrong sid"),
    )
    .expect("wrong sid eval response json");
    assert_eq!(eval["ok"], false);
    assert_eq!(eval["error_code"], "lease_sid_mismatch");
}

/// Verify runtime sessions reject a mismatched echoed generation before executing code.
/// 验证运行时会话会在执行前拒绝不匹配的回传 generation。
#[test]
fn runtime_session_eval_rejects_generation_mismatch() {
    let engine = make_runtime_test_engine();
    let created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"generation-test","ttl_sec":60}"#)
            .expect("create generation runtime session"),
    )
    .expect("generation create response json");
    let lease_id = created["lease_id"]
        .as_str()
        .expect("generation lease id should be present")
        .to_string();
    let sid = created["sid"]
        .as_str()
        .expect("generation sid should be present")
        .to_string();

    let eval_request = json!({
        "lease_id": lease_id,
        "sid": sid,
        "generation": 999_u64,
        "code": "return 1"
    });
    let eval: Value = serde_json::from_str(
        &engine
            .eval_runtime_lease_json(&eval_request.to_string())
            .expect("eval runtime session with wrong generation"),
    )
    .expect("wrong generation eval response json");
    assert_eq!(eval["ok"], false);
    assert_eq!(eval["error_code"], "lease_generation_mismatch");
}

/// Verify runtime-session list only returns active leases and supports SID filtering.
/// 验证运行时会话列表仅返回活跃租约并支持 SID 过滤。
#[test]
fn runtime_session_list_returns_only_active_leases() {
    let engine = make_runtime_test_engine();
    let alpha_created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"alpha-test","ttl_sec":60}"#)
            .expect("create alpha runtime session"),
    )
    .expect("alpha create response json");
    let beta_created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"beta-test","ttl_sec":60}"#)
            .expect("create beta runtime session"),
    )
    .expect("beta create response json");
    let beta_lease_id = beta_created["lease_id"]
        .as_str()
        .expect("beta lease id should be present")
        .to_string();

    let all_list: Value = serde_json::from_str(
        &engine
            .list_runtime_leases_json(r#"{}"#)
            .expect("list runtime sessions"),
    )
    .expect("list response json");
    assert_eq!(all_list["ok"], true);
    assert_eq!(all_list["leases"].as_array().map(Vec::len), Some(2),);

    let alpha_only: Value = serde_json::from_str(
        &engine
            .list_runtime_leases_json(r#"{"sid":"alpha-test"}"#)
            .expect("list alpha runtime sessions"),
    )
    .expect("alpha list response json");
    assert_eq!(alpha_only["ok"], true);
    assert_eq!(alpha_only["leases"].as_array().map(Vec::len), Some(1),);
    assert_eq!(alpha_only["leases"][0]["sid"], alpha_created["sid"]);

    let beta_close_request = json!({ "lease_id": beta_lease_id });
    let beta_closed: Value = serde_json::from_str(
        &engine
            .close_runtime_lease_json(&beta_close_request.to_string())
            .expect("close beta runtime session"),
    )
    .expect("beta close response json");
    assert_eq!(beta_closed["ok"], true);

    let remaining: Value = serde_json::from_str(
        &engine
            .list_runtime_leases_json(r#"{}"#)
            .expect("list remaining runtime sessions"),
    )
    .expect("remaining list response json");
    assert_eq!(remaining["ok"], true);
    assert_eq!(remaining["leases"].as_array().map(Vec::len), Some(1),);
    assert_eq!(remaining["leases"][0]["sid"], alpha_created["sid"]);
}

/// Verify list requests still return busy active leases while a caller is holding the session lock.
/// 验证当调用方持有会话锁时列表请求仍然会返回忙碌但活跃的租约。
#[test]
fn runtime_session_list_keeps_busy_active_leases_visible() {
    let engine = make_runtime_test_engine();
    let created: Value = serde_json::from_str(
        &engine
            .create_runtime_lease_json(r#"{"sid":"busy-list-test","ttl_sec":60}"#)
            .expect("create busy runtime session"),
    )
    .expect("busy create response json");
    let lease_id = created["lease_id"]
        .as_str()
        .expect("busy lease id should be present")
        .to_string();
    let session = engine
        .runtime_sessions
        .get(&lease_id, None, None, None)
        .expect("get busy runtime session");
    let _guard = session.lock().expect("lock busy runtime session");

    let listed: Value = serde_json::from_str(
        &engine
            .list_runtime_leases_json(r#"{"sid":"busy-list-test"}"#)
            .expect("list busy runtime sessions"),
    )
    .expect("busy list response json");
    assert_eq!(listed["ok"], true);
    assert_eq!(listed["leases"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed["leases"][0]["lease_id"], lease_id);
}

/// Verify that run_lua clears transient args after one failed execution.
/// 验证 run_lua 在失败执行后同样会清理临时参数状态。
#[test]
fn run_lua_clears_args_after_failure() {
    let engine = make_runtime_test_engine();
    let error = engine
        .run_lua("error('boom')", &json!({"value":"hello"}), None)
        .expect_err("run_lua should fail");
    assert!(error.contains("Lua run_lua error"));

    let lease = engine.acquire_vm().expect("reacquire pooled vm");
    assert_vm_scope_is_clean(lease.lua());
}

/// Verify that `vulcan.call` restores the outer execution context even when the nested skill corrupts it.
/// 验证当嵌套技能破坏上下文时，`vulcan.call` 仍会恢复外层执行上下文。
#[test]
fn vulcan_call_restores_outer_context_after_nested_failure() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_nested_call_restore_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = fs::remove_dir_all(&temp_root);
    }
    let skill_root = temp_root.join("skills");
    let skill_dir = skill_root.join("test-skill");
    fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime dir");
    fs::write(
            skill_dir.join("skill.yaml"),
            "name: test-skill\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: outer\n    lua_entry: runtime/outer.lua\n    lua_module: test-skill.outer\n  - name: nested\n    lua_entry: runtime/nested.lua\n    lua_module: test-skill.nested\n",
        )
        .expect("write skill yaml");
    fs::write(
            skill_dir.join("runtime").join("outer.lua"),
            "return function(args)\n  local ok, err = pcall(vulcan.call, \"test-skill-nested\", {})\n  if ok then\n    return \"nested-call-unexpected-success\"\n  end\n  local tool_name = (vulcan.runtime and vulcan.runtime.internal and vulcan.runtime.internal.tool_name) or \"tool-nil\"\n  local entry_file = (vulcan.context and vulcan.context.entry_file) or \"entry-nil\"\n  local deps_path = (vulcan.deps and vulcan.deps.lua_path) or \"deps-nil\"\n  return tool_name .. \"|\" .. entry_file .. \"|\" .. deps_path\nend\n",
        )
        .expect("write outer runtime entry");
    fs::write(
            skill_dir.join("runtime").join("nested.lua"),
            "return function(args)\n  vulcan.runtime = nil\n  vulcan.context = nil\n  vulcan.deps = nil\n  error(\"boom\")\nend\n",
        )
        .expect("write nested runtime entry");

    let mut engine = LuaEngine::new(LuaEngineOptions {
        host_options: LuaRuntimeHostOptions::default(),
        pool_config: LuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 60,
        },
    })
    .expect("create engine");
    engine
        .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: skill_root.clone(),
        }])
        .expect("load nested-call test skill");

    let result = engine
        .call_skill("test-skill-outer", &json!({}), None)
        .expect("outer skill should succeed after nested failure");
    assert!(result.content.starts_with("test-skill-outer|"));
    assert!(result.content.contains("outer.lua"));
    assert!(!result.content.contains("|entry-nil|"));
    assert!(!result.content.ends_with("|deps-nil"));
    assert!(result.content.contains("test-skill"));

    let _ = fs::remove_dir_all(&temp_root);
}
