package luaskills

// SkillManagementClient is the ordinary or system lifecycle namespace over JSON FFI management entrypoints.
// SkillManagementClient 是覆盖 JSON FFI 管理入口的普通或 system 生命周期命名空间。
type SkillManagementClient struct {
	client      *Client
	systemPlane bool
	authority   Authority
}

// SystemSkillManagementClient is the system lifecycle namespace with host-injected authority.
// SystemSkillManagementClient 是携带宿主注入权限的 system 生命周期命名空间。
type SystemSkillManagementClient struct {
	SkillManagementClient
}

// Disable disables one skill through formal root-chain lifecycle state.
// Disable 通过正式 root 链生命周期状态停用单个 skill。
func (m *SkillManagementClient) Disable(skillRoots []RuntimeSkillRoot, skillID string, reason string) (map[string]any, error) {
	var result map[string]any
	payload := map[string]any{
		"engine_id":   m.client.EngineID,
		"skill_roots": skillRoots,
		"skill_id":    skillID,
		"reason":      nil,
	}
	if reason != "" {
		payload["reason"] = reason
	}
	m.addAuthority(payload, "")
	err := m.client.call(m.functionName("disable_skill"), payload, &result)
	return result, err
}

// Enable enables one skill through formal root-chain lifecycle state.
// Enable 通过正式 root 链生命周期状态启用单个 skill。
func (m *SkillManagementClient) Enable(skillRoots []RuntimeSkillRoot, skillID string) (map[string]any, error) {
	var result map[string]any
	payload := map[string]any{
		"engine_id":   m.client.EngineID,
		"skill_roots": skillRoots,
		"skill_id":    skillID,
	}
	m.addAuthority(payload, "")
	err := m.client.call(m.functionName("enable_skill"), payload, &result)
	return result, err
}

// Install installs one managed skill through the current lifecycle namespace.
// Install 通过当前生命周期命名空间安装单个受管 skill。
func (m *SkillManagementClient) Install(skillRoots []RuntimeSkillRoot, request SkillInstallRequest, options LifecycleOptions) (map[string]any, error) {
	return m.apply("install_skill", skillRoots, request, options)
}

// Update updates one managed skill through the current lifecycle namespace.
// Update 通过当前生命周期命名空间更新单个受管 skill。
func (m *SkillManagementClient) Update(skillRoots []RuntimeSkillRoot, request SkillInstallRequest, options LifecycleOptions) (map[string]any, error) {
	return m.apply("update_skill", skillRoots, request, options)
}

// Uninstall uninstalls one skill and optionally cleans its databases.
// Uninstall 卸载单个 skill，并可选清理其数据库。
func (m *SkillManagementClient) Uninstall(skillRoots []RuntimeSkillRoot, skillID string, uninstallOptions SkillUninstallOptions, lifecycleOptions LifecycleOptions) (map[string]any, error) {
	var result map[string]any
	payload := map[string]any{
		"engine_id":   m.client.EngineID,
		"skill_roots": skillRoots,
		"skill_id":    skillID,
		"options":     uninstallOptions,
		"target_root": lifecycleOptions.TargetRoot,
	}
	m.addAuthority(payload, lifecycleOptions.Authority)
	err := m.client.call(m.functionName("uninstall_skill"), payload, &result)
	return result, err
}

// apply executes one install or update JSON FFI action.
// apply 执行单个 install 或 update JSON FFI 动作。
func (m *SkillManagementClient) apply(actionName string, skillRoots []RuntimeSkillRoot, request SkillInstallRequest, options LifecycleOptions) (map[string]any, error) {
	var result map[string]any
	payload := map[string]any{
		"engine_id":   m.client.EngineID,
		"skill_roots": skillRoots,
		"request":     request,
		"target_root": options.TargetRoot,
	}
	m.addAuthority(payload, options.Authority)
	err := m.client.call(m.functionName(actionName), payload, &result)
	return result, err
}

// functionName builds the concrete JSON FFI function name for the current namespace.
// functionName 为当前命名空间构造具体 JSON FFI 函数名称。
func (m *SkillManagementClient) functionName(baseName string) string {
	if m.systemPlane {
		return "luaskills_ffi_system_" + baseName + "_json"
	}
	return "luaskills_ffi_" + baseName + "_json"
}

// addAuthority adds authority when the current namespace targets system entrypoints.
// addAuthority 在当前命名空间指向 system 入口时添加权限。
func (m *SkillManagementClient) addAuthority(payload map[string]any, override Authority) {
	if !m.systemPlane {
		return
	}
	authority := override
	if authority == "" {
		authority = m.authority
	}
	if authority == "" {
		authority = AuthoritySystem
	}
	payload["authority"] = authority
}
