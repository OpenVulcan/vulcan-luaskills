/**
 * Host-injected authority used by visibility queries and system management calls.
 * 可见性查询与 system 管理调用使用的宿主注入权限。
 */
export var Authority;
(function (Authority) {
    /**
     * Full host authority that may manage the ROOT layer.
     * 可管理 ROOT 层的完整宿主权限。
     */
    Authority["System"] = "system";
    /**
     * Delegated tool authority that follows ordinary user-facing boundaries.
     * 遵守普通用户可见边界的委托工具权限。
     */
    Authority["DelegatedTool"] = "delegated_tool";
})(Authority || (Authority = {}));
/**
 * Supported managed skill source type.
 * 支持的受管 skill 来源类型。
 */
export var SkillInstallSourceType;
(function (SkillInstallSourceType) {
    /**
     * GitHub Release backed managed skill.
     * 基于 GitHub Release 的受管 skill。
     */
    SkillInstallSourceType["Github"] = "github";
    /**
     * Remote source descriptor URL.
     * 远程 source 描述文件 URL。
     */
    SkillInstallSourceType["Url"] = "url";
})(SkillInstallSourceType || (SkillInstallSourceType = {}));
//# sourceMappingURL=types.js.map