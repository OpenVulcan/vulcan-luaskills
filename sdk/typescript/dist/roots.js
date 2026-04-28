import { mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
/**
 * Helper utilities for building the formal ROOT, PROJECT, USER skill-root chain.
 * 用于构造正式 ROOT、PROJECT、USER 技能根链的辅助工具。
 */
export class RuntimeRoots {
    /**
     * Build one standard formal root chain from a shared runtime root.
     * 基于共享 runtime root 构造一条标准正式根链。
     */
    static standard(options) {
        const normalizedOptions = typeof options === "string" ? { runtimeRoot: options } : options;
        const runtimeRoot = resolve(normalizedOptions.runtimeRoot);
        const roots = [
            {
                name: "ROOT",
                skills_dir: join(runtimeRoot, normalizedOptions.rootSkillsDirName ?? "root_skills"),
            },
        ];
        if (normalizedOptions.includeProject ?? true) {
            roots.push({
                name: "PROJECT",
                skills_dir: join(runtimeRoot, normalizedOptions.projectSkillsDirName ?? "project_skills"),
            });
        }
        if (normalizedOptions.includeUser ?? true) {
            roots.push({
                name: "USER",
                skills_dir: join(runtimeRoot, normalizedOptions.userSkillsDirName ?? "user_skills"),
            });
        }
        return roots;
    }
    /**
     * Build one ROOT-only chain for hosts that intentionally expose no ordinary skill layer.
     * 为明确不暴露普通 skill 层的宿主构造一条仅 ROOT 的根链。
     */
    static rootOnly(runtimeRoot) {
        return RuntimeRoots.standard({
            runtimeRoot,
            includeProject: false,
            includeUser: false,
        });
    }
    /**
     * Find one root by formal label using the same trim and uppercase convention as the runtime.
     * 按运行时相同的 trim 与大写约定查找单个正式标签 root。
     */
    static findByLabel(skillRoots, label) {
        const normalizedLabel = RuntimeRoots.normalizeLabel(label);
        const root = skillRoots.find((candidate) => RuntimeRoots.normalizeLabel(candidate.name) === normalizedLabel);
        if (!root) {
            throw new Error(`Runtime skill root '${normalizedLabel}' is not present in the configured root chain`);
        }
        return root;
    }
    /**
     * Create runtime directories needed by the default SDK host options and root chain.
     * 创建默认 SDK 宿主选项和 root 链所需的运行时目录。
     */
    static ensureLayout(runtimeRoot, skillRoots = RuntimeRoots.standard(runtimeRoot)) {
        const absoluteRoot = resolve(runtimeRoot);
        const directories = [
            absoluteRoot,
            join(absoluteRoot, "temp"),
            join(absoluteRoot, "temp", "downloads"),
            join(absoluteRoot, "resources"),
            join(absoluteRoot, "lua_packages"),
            join(absoluteRoot, "bin", "tools"),
            join(absoluteRoot, "libs"),
            join(absoluteRoot, "dependencies"),
            join(absoluteRoot, "state"),
            join(absoluteRoot, "databases"),
            ...skillRoots.map((root) => root.skills_dir),
        ];
        for (const directory of directories) {
            mkdirSync(directory, { recursive: true });
        }
    }
    /**
     * Normalize one formal root label for client-side lookup.
     * 为客户端侧查找归一化单个正式 root 标签。
     */
    static normalizeLabel(label) {
        return label.trim().toUpperCase();
    }
}
//# sourceMappingURL=roots.js.map