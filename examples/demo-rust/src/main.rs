use serde_json::json;
use std::{env, path::PathBuf};
use vulcan_luaskills::{
    LuaEngine, LuaEngineOptions, LuaInvocationContext, LuaRuntimeHostOptions, LuaVmPoolConfig,
    RuntimeSkillRoot,
};

/// Run the direct Rust host demo against the shared standard runtime fixture.
/// 针对共享标准运行期样例执行 Rust 直连宿主 demo。
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Runtime root used by both Rust and FFI demos.
    // Rust 与 FFI demo 共用的运行根目录。
    // Manifest dir is the demo crate root in both repository and packaged layouts.
    // Manifest dir 在仓库布局和发布包布局中都指向 demo crate 根目录。
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Runtime root first honors packaged run scripts, then falls back to local repository fixtures.
    // Runtime root 优先使用发布包运行脚本注入的路径，再回退到本地仓库样例目录。
    let runtime_root = env::var_os("LUASKILLS_RUNTIME_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let packaged_runtime_root = manifest_dir.join("runtime");
            if packaged_runtime_root.exists() {
                packaged_runtime_root
            } else {
                manifest_dir.join("../ffi/standard_runtime/runtime_root")
            }
        });

    // Host options explicitly inject every runtime path instead of letting the library infer host policy.
    // 宿主选项显式注入每个运行期路径，避免让库自行推导宿主策略。
    let mut host_options = LuaRuntimeHostOptions::default();
    host_options.temp_dir = Some(runtime_root.join("temp"));
    host_options.resources_dir = Some(runtime_root.join("resources"));
    host_options.lua_packages_dir = Some(runtime_root.join("lua_packages"));
    host_options.host_provided_lua_root = Some(runtime_root.join("lua_packages"));
    host_options.host_provided_ffi_root = Some(runtime_root.join("libs"));
    host_options.host_provided_tool_root = Some(runtime_root.join("bin").join("tools"));
    host_options.download_cache_root = Some(runtime_root.join("temp").join("downloads"));
    host_options.dependency_dir_name = "dependencies".to_string();
    host_options.state_dir_name = "state".to_string();
    host_options.database_dir_name = "databases".to_string();
    host_options.space_controller.executable_path =
        Some(runtime_root.join("bin").join(vldb_controller_binary_name()));

    // Pool config keeps the demo small while still using the real engine path.
    // 池配置保持 demo 轻量，同时仍走真实引擎路径。
    let pool_config = LuaVmPoolConfig {
        min_size: 1,
        max_size: 2,
        idle_ttl_secs: 30,
    };

    // Engine owns the Lua VM pool and loaded skill registry.
    // Engine 持有 Lua 虚拟机池和已加载 skill 注册表。
    let mut engine = LuaEngine::new(LuaEngineOptions::new(pool_config, host_options))?;

    // Skill roots tell the runtime which directory contains loadable skills.
    // Skill roots 告诉运行时哪个目录包含可加载的技能。
    let skill_roots = [RuntimeSkillRoot {
        name: "DEMO".to_string(),
        skills_dir: runtime_root.join("skills"),
    }];

    engine.load_from_roots(&skill_roots)?;

    // Invocation context is intentionally empty so the demo focuses on loading and calling.
    // 调用上下文刻意保持为空，使 demo 聚焦加载与调用流程。
    let invocation_context = LuaInvocationContext::empty();
    let result = engine.call_skill(
        "demo-standard-ffi-skill-ping",
        &json!({ "note": "rust" }),
        Some(&invocation_context),
    )?;

    println!("{}", result.content);
    Ok(())
}

/// Return the platform-specific vldb-controller executable name.
/// 返回平台相关的 vldb-controller 可执行文件名。
fn vldb_controller_binary_name() -> &'static str {
    if cfg!(windows) {
        "vldb-controller.exe"
    } else {
        "vldb-controller"
    }
}
