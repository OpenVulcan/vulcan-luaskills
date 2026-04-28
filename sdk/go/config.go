package luaskills

// ConfigClient is the skill-config namespace backed by the unified runtime config store.
// ConfigClient 是基于统一运行时配置存储的 skill 配置命名空间。
type ConfigClient struct {
	client *Client
}

// List lists flattened config records, optionally limited to one skill id.
// List 列出扁平化配置记录，并可选限制到单个 skill id。
func (c *ConfigClient) List(skillID string) ([]map[string]any, error) {
	var result []map[string]any
	payload := map[string]any{"engine_id": c.client.EngineID}
	if skillID != "" {
		payload["skill_id"] = skillID
	}
	err := c.client.call("luaskills_ffi_skill_config_list_json", payload, &result)
	return result, err
}

// Get reads one config value by skill id and key.
// Get 按 skill id 与 key 读取单个配置值。
func (c *ConfigClient) Get(skillID string, key string) (map[string]any, error) {
	var result map[string]any
	err := c.client.call("luaskills_ffi_skill_config_get_json", map[string]any{
		"engine_id": c.client.EngineID,
		"skill_id":  skillID,
		"key":       key,
	}, &result)
	return result, err
}

// Set writes one config value by skill id and key.
// Set 按 skill id 与 key 写入单个配置值。
func (c *ConfigClient) Set(skillID string, key string, value string) (map[string]any, error) {
	var result map[string]any
	err := c.client.call("luaskills_ffi_skill_config_set_json", map[string]any{
		"engine_id": c.client.EngineID,
		"skill_id":  skillID,
		"key":       key,
		"value":     value,
	}, &result)
	return result, err
}

// Delete removes one config value by skill id and key.
// Delete 按 skill id 与 key 删除单个配置值。
func (c *ConfigClient) Delete(skillID string, key string) (map[string]any, error) {
	var result map[string]any
	err := c.client.call("luaskills_ffi_skill_config_delete_json", map[string]any{
		"engine_id": c.client.EngineID,
		"skill_id":  skillID,
		"key":       key,
	}, &result)
	return result, err
}
