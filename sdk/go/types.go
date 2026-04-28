package luaskills

// Authority is the host-injected authority used by query and system management entrypoints.
// Authority 是查询与 system 管理入口使用的宿主注入权限。
type Authority string

const (
	// AuthoritySystem may manage the ROOT layer when used with system entrypoints.
	// AuthoritySystem 搭配 system 入口时可以管理 ROOT 层。
	AuthoritySystem Authority = "system"
	// AuthorityDelegatedTool follows ordinary user-facing visibility and management boundaries.
	// AuthorityDelegatedTool 遵守普通用户侧可见性与管理边界。
	AuthorityDelegatedTool Authority = "delegated_tool"
)

// SkillInstallSourceType is the managed source type used by install and update requests.
// SkillInstallSourceType 是 install 与 update 请求使用的受管来源类型。
type SkillInstallSourceType string

const (
	// SkillInstallSourceGithub resolves one managed skill from GitHub release metadata.
	// SkillInstallSourceGithub 通过 GitHub release 元数据解析受管 skill。
	SkillInstallSourceGithub SkillInstallSourceType = "github"
	// SkillInstallSourceURL resolves one managed skill from one remote source descriptor URL.
	// SkillInstallSourceURL 通过远程 source 描述文件 URL 解析受管 skill。
	SkillInstallSourceURL SkillInstallSourceType = "url"
)

// RuntimeSkillRoot is one named runtime skill root in the formal ROOT, PROJECT, USER chain.
// RuntimeSkillRoot 是正式 ROOT、PROJECT、USER 链中的单个命名运行时 skill root。
type RuntimeSkillRoot struct {
	Name      string `json:"name"`
	SkillsDir string `json:"skills_dir"`
}

// InvocationContext is the optional context injected into call_skill and run_lua.
// InvocationContext 是注入 call_skill 与 run_lua 的可选上下文。
type InvocationContext struct {
	RequestContext any `json:"request_context,omitempty"`
	ClientBudget   any `json:"client_budget,omitempty"`
	ToolConfig     any `json:"tool_config,omitempty"`
}

// SkillInstallRequest is one managed install or update request.
// SkillInstallRequest 是单个受管安装或更新请求。
type SkillInstallRequest struct {
	SkillID    *string                `json:"skill_id,omitempty"`
	Source     *string                `json:"source,omitempty"`
	SourceType SkillInstallSourceType `json:"source_type,omitempty"`
}

// SkillUninstallOptions controls optional database cleanup after uninstall.
// SkillUninstallOptions 控制卸载后的可选数据库清理。
type SkillUninstallOptions struct {
	RemoveSQLite  bool `json:"remove_sqlite,omitempty"`
	RemoveLanceDB bool `json:"remove_lancedb,omitempty"`
}

// LifecycleOptions carries optional target-root and authority overrides.
// LifecycleOptions 携带可选 target-root 与 authority 覆盖。
type LifecycleOptions struct {
	TargetRoot *RuntimeSkillRoot `json:"target_root,omitempty"`
	Authority  Authority         `json:"authority,omitempty"`
}

// RuntimeInvocationResult is the JSON FFI result returned by call_skill.
// RuntimeInvocationResult 是 call_skill 返回的 JSON FFI 结果。
type RuntimeInvocationResult struct {
	Content      string  `json:"content"`
	OverflowMode *string `json:"overflow_mode"`
	TemplateHint *string `json:"template_hint"`
	ContentBytes int     `json:"content_bytes"`
	ContentLines int     `json:"content_lines"`
}

// ClientOptions controls creation of one LuaSkills client and native engine.
// ClientOptions 控制单个 LuaSkills 客户端与原生引擎的创建。
type ClientOptions struct {
	RuntimeRoot         string
	EngineOptions       map[string]any
	HostOptions         map[string]any
	PoolConfig          map[string]any
	EnsureRuntimeLayout bool
}
