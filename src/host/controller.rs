use crate::host::database::RuntimeDatabaseBindingContext;
use crate::host::options::{LuaRuntimeHostOptions, LuaRuntimeSpaceControllerProcessMode};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::{Handle, Runtime};
use vldb_controller_client::{
    ClientRegistration, ControllerClient, ControllerClientConfig, ControllerProcessMode, SpaceKind,
    SpaceRegistration,
};

/// Shared host-side controller bridge that executes async controller SDK calls from sync runtime code.
/// 供同步运行时代码调用异步控制器 SDK 的共享宿主桥接。
pub struct LuaRuntimeSpaceControllerBridge {
    client: ControllerClient,
    runtime: Mutex<Runtime>,
}

impl LuaRuntimeSpaceControllerBridge {
    /// Build one controller bridge from host options and one stable backend-specific registration suffix.
    /// 基于宿主选项与稳定的后端注册后缀构建一个控制器桥接。
    pub fn new(
        host_options: &LuaRuntimeHostOptions,
        backend_suffix: &str,
    ) -> Result<Arc<Self>, String> {
        let controller_options = &host_options.space_controller;
        let endpoint = controller_options
            .endpoint
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:19801".to_string());
        let process_id = std::process::id();
        let started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        let registration = ClientRegistration {
            client_id: format!(
                "vulcan-luaskills-{}-{}-{}",
                process_id, backend_suffix, started_at_ms
            ),
            host_kind: "vulcan_luaskills".to_string(),
            process_id,
            process_name: backend_suffix.to_string(),
            lease_ttl_secs: Some(controller_options.default_lease_ttl_secs),
        };
        let config = ControllerClientConfig {
            endpoint,
            auto_spawn: controller_options.auto_spawn,
            spawn_executable: controller_options
                .executable_path
                .as_ref()
                .map(|path| path.to_string_lossy().to_string()),
            spawn_process_mode: map_process_mode(controller_options.process_mode),
            minimum_uptime_secs: controller_options.minimum_uptime_secs,
            idle_timeout_secs: controller_options.idle_timeout_secs,
            default_lease_ttl_secs: controller_options.default_lease_ttl_secs,
            connect_timeout_secs: controller_options.connect_timeout_secs,
            startup_timeout_secs: controller_options.startup_timeout_secs,
            startup_retry_interval_ms: controller_options.startup_retry_interval_ms,
            lease_renew_interval_secs: controller_options.lease_renew_interval_secs,
        };
        let runtime = Runtime::new()
            .map_err(|error| format!("failed to create controller tokio runtime: {}", error))?;
        let client = ControllerClient::new(config, registration);
        run_controller_operation_with_client(&runtime, &client, |client| async move {
            client.connect().await
        })
            .map_err(|error| format!("failed to connect space controller client: {}", error))?;
        Ok(Arc::new(Self {
            client,
            runtime: Mutex::new(runtime),
        }))
    }

    /// Execute one controller SDK operation while transparently handling sync and async host threads.
    /// 透明兼容同步线程与异步宿主线程，执行一次控制器 SDK 操作。
    pub fn run<F, Fut, T>(&self, operation: F) -> Result<T, String>
    where
        F: FnOnce(ControllerClient) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, vldb_controller_client::client::BoxError>> + Send + 'static,
        T: Send + 'static,
    {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "controller runtime lock poisoned".to_string())?;
        run_controller_operation_with_client(&runtime, &self.client, operation)
            .map_err(|error| format!("space controller request failed: {}", error))
    }

    /// Attach one stable binding context as one controller space before backend operations start.
    /// 在后端操作开始前，把稳定绑定上下文附着为一个控制器空间。
    pub fn attach_binding(&self, binding: &RuntimeDatabaseBindingContext) -> Result<(), String> {
        let registration = SpaceRegistration {
            space_id: controller_space_id_for_binding(binding),
            space_label: binding.space_label.clone(),
            space_kind: map_space_kind(&binding.space_label),
            space_root: binding.space_root.clone(),
        };
        self.run(move |client| async move { client.attach_space(registration).await })
            .map(|_| ())
    }
}

/// Execute one controller SDK operation safely from both sync code and threads already inside a Tokio runtime.
/// 兼容同步代码与已处于 Tokio 运行时中的线程，安全执行一次控制器 SDK 操作。
fn run_controller_operation_with_client<F, Fut, T>(
    runtime: &Runtime,
    client: &ControllerClient,
    operation: F,
) -> Result<T, vldb_controller_client::client::BoxError>
where
    F: FnOnce(ControllerClient) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, vldb_controller_client::client::BoxError>> + Send + 'static,
    T: Send + 'static,
{
    let client_clone = client.clone();
    if let Ok(handle) = Handle::try_current() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        tokio::task::block_in_place(|| {
            handle.spawn(async move {
                let result = operation(client_clone).await;
                let _ = sender.send(result);
            });
            receiver
                .recv()
                .unwrap_or_else(|_| Err("space controller task channel closed".into()))
        })
    } else {
        runtime.block_on(operation(client_clone))
    }
}

impl Drop for LuaRuntimeSpaceControllerBridge {
    /// Best-effort shutdown the controller client when the bridge goes away.
    /// 在桥接析构时尽力关闭控制器客户端。
    fn drop(&mut self) {
        let client = self.client.clone();
        let _ = thread::Builder::new()
            .name("vulcan-space-controller-shutdown".to_string())
            .spawn(move || {
                let Ok(runtime) = Runtime::new() else {
                    return;
                };
                let _ = runtime.block_on(async move {
                    let _ =
                        tokio::time::timeout(Duration::from_millis(250), client.shutdown()).await;
                });
            });
    }
}

/// Map the host-facing process mode into the controller client SDK process mode.
/// 把宿主侧进程模式映射成控制器客户端 SDK 进程模式。
fn map_process_mode(mode: LuaRuntimeSpaceControllerProcessMode) -> ControllerProcessMode {
    match mode {
        LuaRuntimeSpaceControllerProcessMode::Service => ControllerProcessMode::Service,
        LuaRuntimeSpaceControllerProcessMode::Managed => ControllerProcessMode::Managed,
    }
}

/// Map one stable host space label into the controller SDK logical space kind.
/// 把稳定宿主空间标签映射成控制器 SDK 逻辑空间类型。
fn map_space_kind(space_label: &str) -> SpaceKind {
    match space_label.trim().to_ascii_uppercase().as_str() {
        "ROOT" => SpaceKind::Root,
        "USER" => SpaceKind::User,
        _ => SpaceKind::Project,
    }
}

/// Build the stable runtime-space identity used by the shared controller for one binding context.
/// 为单个绑定上下文构建供共享控制器使用的稳定运行时空间标识。
pub fn controller_space_id_for_binding(binding: &RuntimeDatabaseBindingContext) -> String {
    let normalized_label = normalize_controller_space_label(&binding.space_label);
    let mut digest = Sha256::new();
    digest.update(binding.space_label.trim().as_bytes());
    digest.update([0]);
    digest.update(binding.space_root.as_bytes());
    let hash_hex = format!("{:x}", digest.finalize());
    format!("{}-{}", normalized_label, &hash_hex[..16])
}

/// Normalize one host-provided space label into a controller-safe identifier prefix.
/// 将宿主提供的空间标签标准化为控制器安全的标识符前缀。
fn normalize_controller_space_label(space_label: &str) -> String {
    let normalized: String = space_label
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if normalized.is_empty() {
        "SPACE".to_string()
    } else {
        normalized
    }
}
