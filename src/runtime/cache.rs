use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Default maximum number of cached entries for the process-wide shared tool cache.
/// 工具缓存默认最大条目数，用于限制进程内共享缓存的总体容量。
pub const DEFAULT_TOOL_CACHE_MAX_ENTRIES: usize = 1000;

/// Default cache entry lifetime in seconds, used when callers do not provide an explicit TTL.
/// 工具缓存默认存活时间，单位为秒；未显式指定时使用该值。
pub const DEFAULT_TOOL_CACHE_DEFAULT_TTL_SECS: u64 = 30 * 60;

/// Maximum cache entry lifetime in seconds; larger requested TTL values are clamped to this ceiling.
/// 工具缓存允许的最长存活时间，单位为秒；超过该值会被自动钳制。
pub const DEFAULT_TOOL_CACHE_MAX_TTL_SECS: u64 = 30 * 60;

/// Runtime configuration for the shared tool cache, controlling capacity and expiration behavior.
/// 共享工具缓存的运行时配置，控制容量与过期策略。
#[derive(Clone, Debug)]
pub struct ToolCacheConfig {
    /// Maximum number of entries; oldest entries are evicted when the cache exceeds this size.
    /// 缓存最大条目数，超出后会按创建顺序淘汰最旧条目。
    pub max_entries: usize,
    /// Default TTL in seconds used when callers omit a TTL.
    /// 默认 TTL（秒），调用方未传 TTL 时使用。
    pub default_ttl_secs: u64,
    /// Maximum TTL in seconds; requested TTL values are clamped to this ceiling.
    /// 最大 TTL（秒），请求 TTL 会被限制在该范围内。
    pub max_ttl_secs: u64,
}

impl Default for ToolCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_TOOL_CACHE_MAX_ENTRIES,
            default_ttl_secs: DEFAULT_TOOL_CACHE_DEFAULT_TTL_SECS,
            max_ttl_secs: DEFAULT_TOOL_CACHE_MAX_TTL_SECS,
        }
    }
}

/// Internal representation of one cache entry, recording the owning tool, payload, creation order, and expiration time.
/// 单个缓存条目的内部表示，记录归属工具、内容、创建顺序与过期时间。
#[derive(Clone, Debug)]
struct ToolCacheEntry {
    /// Tool or skill name that owns this entry, used to isolate cache namespaces.
    /// 写入该条目的工具/技能名称，用于隔离不同工具的缓存空间。
    tool_name: String,
    /// Cached JSON payload returned to callers as-is on reads.
    /// 缓存的 JSON 值，会在读取时原样返回给调用方。
    value: Value,
    /// Monotonic creation sequence used to evict the oldest entry when capacity is exceeded.
    /// 创建序号，用于在容量超限时淘汰最旧条目。
    created_seq: u64,
    /// Expiration instant; reads after this moment trigger automatic cleanup.
    /// 条目失效时刻；超过该时间后读取会自动清理。
    expires_at: Instant,
}

/// Mutable storage backing the shared cache, protected by a read-write lock.
/// 共享缓存的可变存储体，受读写锁保护。
#[derive(Default)]
struct ToolCacheStore {
    entries: HashMap<String, ToolCacheEntry>,
}

/// Process-wide shared cache for all Lua skills, intended for short-lived pagination and tool state handoff.
/// 主程序级共享工具缓存，供所有 Lua 技能复用短时分页/状态数据。
pub struct SharedToolCache {
    store: RwLock<ToolCacheStore>,
    config: ToolCacheConfig,
    counter: AtomicU64,
}

impl SharedToolCache {
    /// Create a shared cache instance with the provided configuration.
    /// 使用指定配置创建共享缓存实例。
    pub fn new(config: ToolCacheConfig) -> Self {
        Self {
            store: RwLock::new(ToolCacheStore::default()),
            config,
            counter: AtomicU64::new(1),
        }
    }

    /// Store one cache record; missing TTL falls back to the default and values above the ceiling are clamped.
    /// 写入一条缓存记录；TTL 为空时使用默认值，超出上限时会自动钳制。
    pub fn create(&self, tool_name: &str, value: Value, ttl_secs: Option<u64>) -> String {
        let now = Instant::now();
        let ttl = self.resolve_ttl(ttl_secs);
        let cache_id = self.next_cache_id();
        let entry = ToolCacheEntry {
            tool_name: tool_name.to_string(),
            value,
            created_seq: self.counter.fetch_add(1, Ordering::Relaxed),
            expires_at: now + ttl,
        };

        let mut store = self.store.write().expect("tool cache poisoned");
        self.cleanup_expired_locked(&mut store, now);
        store.entries.insert(cache_id.clone(), entry);
        self.enforce_capacity_locked(&mut store);
        cache_id
    }

    /// Read a cached entry by tool name and cache id; expired hits are removed and returned as empty.
    /// 按工具名和缓存编号读取缓存；命中但已过期时会自动删除并返回空。
    pub fn get(&self, tool_name: &str, cache_id: &str) -> Option<Value> {
        let now = Instant::now();

        {
            let store = self.store.read().expect("tool cache poisoned");
            if let Some(entry) = store.entries.get(cache_id)
                && entry.tool_name == tool_name
                && entry.expires_at > now
            {
                return Some(entry.value.clone());
            }
        }

        let mut store = self.store.write().expect("tool cache poisoned");
        self.cleanup_expired_locked(&mut store, now);
        match store.entries.get(cache_id) {
            Some(entry) if entry.tool_name == tool_name && entry.expires_at > now => {
                Some(entry.value.clone())
            }
            _ => None,
        }
    }

    /// Delete one cache entry under the given tool namespace and return whether an entry was actually removed.
    /// 删除指定工具名下的缓存条目；返回是否确实删除了条目。
    pub fn delete(&self, tool_name: &str, cache_id: &str) -> bool {
        let mut store = self.store.write().expect("tool cache poisoned");
        self.cleanup_expired_locked(&mut store, Instant::now());
        if let Some(entry) = store.entries.get(cache_id)
            && entry.tool_name == tool_name
        {
            store.entries.remove(cache_id);
            return true;
        }
        false
    }

    /// Resolve the effective TTL by applying defaulting, clamping to the configured maximum, and enforcing a 1-second minimum.
    /// 解析最终 TTL，未传使用默认值，超限后按最大值裁剪，最小保证为 1 秒。
    fn resolve_ttl(&self, ttl_secs: Option<u64>) -> Duration {
        let requested = ttl_secs.unwrap_or(self.config.default_ttl_secs);
        let clamped = requested.max(1).min(self.config.max_ttl_secs.max(1));
        Duration::from_secs(clamped)
    }

    /// Generate a cache id by combining a timestamp with a monotonic counter to reduce collision risk.
    /// 生成缓存编号，结合时间戳与自增计数以降低碰撞风险。
    fn next_cache_id(&self) -> String {
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("tc-{}-{}", unix_ms, seq)
    }

    /// Remove all expired entries so subsequent reads and writes operate on the current valid view.
    /// 清理所有已过期条目，保证后续读写看到的是当前有效视图。
    fn cleanup_expired_locked(&self, store: &mut ToolCacheStore, now: Instant) {
        store.entries.retain(|_, entry| entry.expires_at > now);
    }

    /// Evict the oldest entries while the cache is above its configured capacity.
    /// 在缓存超出上限时淘汰最旧条目，直到条目数回落到配置范围内。
    fn enforce_capacity_locked(&self, store: &mut ToolCacheStore) {
        while store.entries.len() > self.config.max_entries.max(1) {
            let oldest_id = store
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.created_seq)
                .map(|(cache_id, _)| cache_id.clone());
            match oldest_id {
                Some(cache_id) => {
                    store.entries.remove(&cache_id);
                }
                None => break,
            }
        }
    }
}

static GLOBAL_TOOL_CACHE: OnceLock<Arc<SharedToolCache>> = OnceLock::new();

/// Initialize the global shared cache, typically once during process startup.
/// 初始化全局共享缓存；通常在主程序启动时执行一次。
pub fn configure_global_tool_cache(config: ToolCacheConfig) {
    let _ = GLOBAL_TOOL_CACHE.set(Arc::new(SharedToolCache::new(config)));
}

/// Get the global shared cache, lazily creating it with default settings if startup did not configure it explicitly.
/// 获取全局共享缓存；若尚未初始化则使用默认配置惰性创建。
pub fn global_tool_cache() -> Arc<SharedToolCache> {
    GLOBAL_TOOL_CACHE
        .get_or_init(|| Arc::new(SharedToolCache::new(ToolCacheConfig::default())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::{SharedToolCache, ToolCacheConfig};
    use serde_json::json;
    use std::thread;
    use std::time::Duration;

    /// Build one deterministic cache config used by unit tests.
    /// 为单元测试构造一份稳定可预测的缓存配置。
    fn test_cache_config(max_entries: usize, default_ttl_secs: u64, max_ttl_secs: u64) -> ToolCacheConfig {
        ToolCacheConfig {
            max_entries,
            default_ttl_secs,
            max_ttl_secs,
        }
    }

    /// Verify entries are isolated by tool namespace and cannot be read across scopes.
    /// 验证缓存条目按工具命名空间隔离，不能跨作用域读取。
    #[test]
    fn cache_entries_are_isolated_by_tool_name() {
        let cache = SharedToolCache::new(test_cache_config(10, 5, 5));
        let cache_id = cache.create("skill-a", json!({"value": 1}), None);

        assert_eq!(cache.get("skill-a", &cache_id), Some(json!({"value": 1})));
        assert_eq!(cache.get("skill-b", &cache_id), None);
    }

    /// Verify cache entries expire according to the configured default TTL.
    /// 验证缓存条目会按照配置的默认 TTL 正常过期。
    #[test]
    fn cache_entries_expire_after_default_ttl() {
        let cache = SharedToolCache::new(test_cache_config(10, 1, 1));
        let cache_id = cache.create("skill-a", json!({"value": 1}), None);

        thread::sleep(Duration::from_millis(1100));

        assert_eq!(cache.get("skill-a", &cache_id), None);
    }

    /// Verify requested TTL values are clamped to the configured maximum TTL.
    /// 验证调用方请求的 TTL 会被正确限制到配置允许的最大值。
    #[test]
    fn cache_requested_ttl_is_clamped_to_maximum() {
        let cache = SharedToolCache::new(test_cache_config(10, 5, 1));
        let cache_id = cache.create("skill-a", json!({"value": 1}), Some(60));

        thread::sleep(Duration::from_millis(1100));

        assert_eq!(cache.get("skill-a", &cache_id), None);
    }

    /// Verify the oldest cache entry is evicted when the capacity is exceeded.
    /// 验证缓存容量超限时会淘汰最早创建的条目。
    #[test]
    fn cache_evicts_oldest_entry_when_capacity_is_exceeded() {
        let cache = SharedToolCache::new(test_cache_config(2, 5, 5));
        let first_id = cache.create("skill-a", json!({"value": 1}), None);
        let second_id = cache.create("skill-a", json!({"value": 2}), None);
        let third_id = cache.create("skill-a", json!({"value": 3}), None);

        assert_eq!(cache.get("skill-a", &first_id), None);
        assert_eq!(cache.get("skill-a", &second_id), Some(json!({"value": 2})));
        assert_eq!(cache.get("skill-a", &third_id), Some(json!({"value": 3})));
    }
}
