package luaskills

import (
	"fmt"
	"os"
	"path/filepath"
)

// Client is one high-level LuaSkills SDK client over the public JSON FFI surface.
// Client 是基于公共 JSON FFI 表面的高级 LuaSkills SDK 客户端。
type Client struct {
	EngineID uint64
	Config   *ConfigClient
	Skills   *SkillManagementClient
	closed   bool
}

// NewClient creates one native LuaSkills engine and wraps it in a high-level client.
// NewClient 创建一个原生 LuaSkills 引擎并封装为高级客户端。
func NewClient(options ClientOptions) (*Client, error) {
	runtimeRoot := options.RuntimeRoot
	if runtimeRoot == "" {
		workingDirectory, err := os.Getwd()
		if err != nil {
			return nil, err
		}
		runtimeRoot = filepath.Join(workingDirectory, "luaskills-runtime")
	}
	engineOptions := options.EngineOptions
	if engineOptions == nil {
		engineOptions = CreateEngineOptions(runtimeRoot, options.HostOptions, options.PoolConfig)
		if options.EnsureRuntimeLayout {
			if err := EnsureRuntimeLayout(runtimeRoot, nil); err != nil {
				return nil, err
			}
		}
	}
	var handle struct {
		EngineID uint64 `json:"engine_id"`
	}
	if err := callJSON("luaskills_ffi_engine_new_json", map[string]any{"options": engineOptions}, &handle); err != nil {
		return nil, err
	}
	client := &Client{
		EngineID: handle.EngineID,
	}
	client.Config = &ConfigClient{client: client}
	client.Skills = &SkillManagementClient{client: client}
	return client, nil
}

// System returns one system-management namespace bound to host-injected authority.
// System 返回绑定到宿主注入权限的 system 管理命名空间。
func (c *Client) System(authority Authority) *SystemSkillManagementClient {
	if authority == "" {
		authority = AuthoritySystem
	}
	return &SystemSkillManagementClient{
		SkillManagementClient: SkillManagementClient{
			client:      c,
			systemPlane: true,
			authority:   authority,
		},
	}
}

// LoadFromRoots loads skills from the formal ordered root chain.
// LoadFromRoots 从正式有序 root 链加载 skills。
func (c *Client) LoadFromRoots(skillRoots []RuntimeSkillRoot) (map[string]any, error) {
	var result map[string]any
	err := c.call("luaskills_ffi_load_from_roots_json", map[string]any{
		"engine_id":   c.EngineID,
		"skill_roots": skillRoots,
	}, &result)
	return result, err
}

// ReloadFromRoots reloads skills from the formal ordered root chain.
// ReloadFromRoots 从正式有序 root 链重载 skills。
func (c *Client) ReloadFromRoots(skillRoots []RuntimeSkillRoot) (map[string]any, error) {
	var result map[string]any
	err := c.call("luaskills_ffi_reload_from_roots_json", map[string]any{
		"engine_id":   c.EngineID,
		"skill_roots": skillRoots,
	}, &result)
	return result, err
}

// ListEntries lists runtime entries visible to the selected authority.
// ListEntries 列出指定权限可见的运行时入口。
func (c *Client) ListEntries(authority Authority) ([]map[string]any, error) {
	if authority == "" {
		authority = AuthorityDelegatedTool
	}
	var result []map[string]any
	err := c.call("luaskills_ffi_list_entries_json", map[string]any{
		"engine_id": c.EngineID,
		"authority": authority,
	}, &result)
	return result, err
}

// ListSkillHelp lists runtime help trees visible to the selected authority.
// ListSkillHelp 列出指定权限可见的运行时帮助树。
func (c *Client) ListSkillHelp(authority Authority) ([]map[string]any, error) {
	if authority == "" {
		authority = AuthorityDelegatedTool
	}
	var result []map[string]any
	err := c.call("luaskills_ffi_list_skill_help_json", map[string]any{
		"engine_id": c.EngineID,
		"authority": authority,
	}, &result)
	return result, err
}

// RenderSkillHelpDetail renders one help flow detail visible to the selected authority.
// RenderSkillHelpDetail 渲染指定权限可见的单个帮助流程详情。
func (c *Client) RenderSkillHelpDetail(skillID string, flowName string, authority Authority, requestContext any) (map[string]any, error) {
	if flowName == "" {
		flowName = "main"
	}
	if authority == "" {
		authority = AuthorityDelegatedTool
	}
	var result map[string]any
	err := c.call("luaskills_ffi_render_skill_help_detail_json", map[string]any{
		"engine_id":       c.EngineID,
		"skill_id":        skillID,
		"flow_name":       flowName,
		"request_context": requestContext,
		"authority":       authority,
	}, &result)
	return result, err
}

// PromptArgumentCompletions queries prompt argument completions visible to the selected authority.
// PromptArgumentCompletions 查询指定权限可见的 prompt 参数补全项。
func (c *Client) PromptArgumentCompletions(promptName string, argumentName string, authority Authority) ([]string, error) {
	if authority == "" {
		authority = AuthorityDelegatedTool
	}
	var result []string
	err := c.call("luaskills_ffi_prompt_argument_completions_json", map[string]any{
		"engine_id":     c.EngineID,
		"prompt_name":   promptName,
		"argument_name": argumentName,
		"authority":     authority,
	}, &result)
	return result, err
}

// IsSkill returns whether one canonical tool name is visible as a skill entry.
// IsSkill 返回指定 canonical 工具名是否可见为 skill 入口。
func (c *Client) IsSkill(toolName string, authority Authority) (bool, error) {
	if authority == "" {
		authority = AuthorityDelegatedTool
	}
	var result struct {
		Value bool `json:"value"`
	}
	err := c.call("luaskills_ffi_is_skill_json", map[string]any{
		"engine_id": c.EngineID,
		"tool_name": toolName,
		"authority": authority,
	}, &result)
	return result.Value, err
}

// SkillNameForTool resolves the owning skill id for one visible canonical tool name.
// SkillNameForTool 解析单个可见 canonical 工具名称所属的 skill id。
func (c *Client) SkillNameForTool(toolName string, authority Authority) (*string, error) {
	if authority == "" {
		authority = AuthorityDelegatedTool
	}
	var result struct {
		SkillID *string `json:"skill_id"`
	}
	err := c.call("luaskills_ffi_skill_name_for_tool_json", map[string]any{
		"engine_id": c.EngineID,
		"tool_name": toolName,
		"authority": authority,
	}, &result)
	return result.SkillID, err
}

// CallSkill calls one active skill entry by canonical tool name.
// CallSkill 按 canonical 工具名称调用单个已激活 skill 入口。
func (c *Client) CallSkill(toolName string, args any, invocationContext *InvocationContext) (*RuntimeInvocationResult, error) {
	if args == nil {
		args = map[string]any{}
	}
	var result RuntimeInvocationResult
	err := c.call("luaskills_ffi_call_skill_json", map[string]any{
		"engine_id":          c.EngineID,
		"tool_name":          toolName,
		"args":               args,
		"invocation_context": normalizeInvocationContext(invocationContext),
	}, &result)
	return &result, err
}

// RunLua executes one inline Lua snippet against the active runtime.
// RunLua 针对当前活动运行时执行单段内联 Lua。
func (c *Client) RunLua(code string, args any, invocationContext *InvocationContext) (any, error) {
	if args == nil {
		args = map[string]any{}
	}
	var result any
	err := c.call("luaskills_ffi_run_lua_json", map[string]any{
		"engine_id":          c.EngineID,
		"code":               code,
		"args":               args,
		"invocation_context": normalizeInvocationContext(invocationContext),
	}, &result)
	return result, err
}

// Close releases the native engine handle.
// Close 释放原生引擎句柄。
func (c *Client) Close() (map[string]any, error) {
	if c.closed {
		return nil, nil
	}
	var result map[string]any
	err := callJSON("luaskills_ffi_engine_free_json", map[string]any{"engine_id": c.EngineID}, &result)
	if err != nil {
		return nil, err
	}
	c.closed = true
	return result, nil
}

// call invokes one JSON FFI function after checking the engine state.
// call 检查引擎状态后调用一个 JSON FFI 函数。
func (c *Client) call(functionName string, payload any, out any) error {
	if c.closed {
		return fmt.Errorf("LuaSkills engine %d is already closed", c.EngineID)
	}
	return callJSON(functionName, payload, out)
}

// CreateEngineOptions builds complete engine options from SDK defaults and caller overrides.
// CreateEngineOptions 基于 SDK 默认值和调用方覆盖构造完整引擎选项。
func CreateEngineOptions(runtimeRoot string, hostOptions map[string]any, poolConfig map[string]any) map[string]any {
	return map[string]any{
		"pool_config":  mergeMaps(DefaultPoolConfig(), poolConfig),
		"host_options": mergeHostOptions(DefaultHostOptions(runtimeRoot), hostOptions),
	}
}

// DefaultPoolConfig returns the SDK default VM pool configuration.
// DefaultPoolConfig 返回 SDK 默认虚拟机池配置。
func DefaultPoolConfig() map[string]any {
	return map[string]any{"min_size": 1, "max_size": 4, "idle_ttl_secs": 60}
}

// DefaultHostOptions returns the SDK default host options for one runtime root.
// DefaultHostOptions 返回单个 runtime root 对应的 SDK 默认宿主选项。
func DefaultHostOptions(runtimeRoot string) map[string]any {
	root := normalizePath(runtimeRoot)
	baseOptions := map[string]any{
		"temp_dir":                normalizePath(filepath.Join(root, "temp")),
		"resources_dir":           normalizePath(filepath.Join(root, "resources")),
		"lua_packages_dir":        normalizePath(filepath.Join(root, "lua_packages")),
		"host_provided_tool_root": normalizePath(filepath.Join(root, "bin", "tools")),
		"host_provided_lua_root":  normalizePath(filepath.Join(root, "lua_packages")),
		"host_provided_ffi_root":  normalizePath(filepath.Join(root, "libs")),
		"download_cache_root":     normalizePath(filepath.Join(root, "temp", "downloads")),
		"dependency_dir_name":     "dependencies",
		"state_dir_name":          "state",
		"database_dir_name":       "databases",
		"skill_config_file_path":  nil,
		"allow_network_download":  true,
		"github_base_url":         nil,
		"github_api_base_url":     nil,
		"sqlite_library_path":     nil,
		"sqlite_provider_mode":    "dynamic_library",
		"sqlite_callback_mode":    "standard",
		"lancedb_library_path":    nil,
		"lancedb_provider_mode":   "dynamic_library",
		"lancedb_callback_mode":   "standard",
		"space_controller":        DefaultSpaceControllerOptions(),
		"cache_config":            nil,
		"runlua_pool_config":      nil,
		"reserved_entry_names":    []string{},
		"ignored_skill_ids":       []string{},
		"capabilities":            map[string]any{"enable_skill_management_bridge": false},
	}
	if manifest, err := LoadRuntimeInstallManifest(root); err == nil && manifest != nil {
		return mergeHostOptions(baseOptions, HostOptionsFromRuntimeManifest(manifest))
	}
	return baseOptions
}

// DefaultSpaceControllerOptions returns the SDK default space-controller options.
// DefaultSpaceControllerOptions 返回 SDK 默认 space-controller 选项。
func DefaultSpaceControllerOptions() map[string]any {
	return map[string]any{
		"endpoint":                  nil,
		"auto_spawn":                false,
		"executable_path":           nil,
		"process_mode":              "managed",
		"minimum_uptime_secs":       300,
		"idle_timeout_secs":         900,
		"default_lease_ttl_secs":    120,
		"connect_timeout_secs":      5,
		"startup_timeout_secs":      15,
		"startup_retry_interval_ms": 250,
		"lease_renew_interval_secs": 30,
	}
}

// mergeHostOptions merges caller host overrides over complete default host options.
// mergeHostOptions 将调用方宿主覆盖合并到完整默认宿主选项上。
func mergeHostOptions(base map[string]any, overrides map[string]any) map[string]any {
	merged := mergeMaps(base, overrides)
	if value, ok := overrides["space_controller"].(map[string]any); ok {
		merged["space_controller"] = mergeMaps(base["space_controller"].(map[string]any), value)
	}
	if value, ok := overrides["capabilities"].(map[string]any); ok {
		merged["capabilities"] = mergeMaps(base["capabilities"].(map[string]any), value)
	}
	return merged
}

// mergeMaps returns one shallow copy of base merged with overrides.
// mergeMaps 返回 base 与 overrides 合并后的浅拷贝。
func mergeMaps(base map[string]any, overrides map[string]any) map[string]any {
	merged := make(map[string]any, len(base)+len(overrides))
	for key, value := range base {
		merged[key] = value
	}
	for key, value := range overrides {
		merged[key] = value
	}
	return merged
}

// normalizeInvocationContext converts one optional invocation context into JSON payload form.
// normalizeInvocationContext 将单个可选调用上下文转换为 JSON 载荷形式。
func normalizeInvocationContext(context *InvocationContext) map[string]any {
	if context == nil {
		return nil
	}
	clientBudget := context.ClientBudget
	if clientBudget == nil {
		clientBudget = map[string]any{}
	}
	toolConfig := context.ToolConfig
	if toolConfig == nil {
		toolConfig = map[string]any{}
	}
	return map[string]any{
		"request_context": context.RequestContext,
		"client_budget":   clientBudget,
		"tool_config":     toolConfig,
	}
}
