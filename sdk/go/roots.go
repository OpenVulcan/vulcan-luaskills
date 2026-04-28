package luaskills

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// StandardRoots builds one formal ROOT, PROJECT, USER root chain from a shared runtime root.
// StandardRoots 基于共享 runtime root 构造一条正式 ROOT、PROJECT、USER root 链。
func StandardRoots(runtimeRoot string) []RuntimeSkillRoot {
	return StandardRootsWithOptions(runtimeRoot, true, true)
}

// StandardRootsWithOptions builds one formal root chain with optional PROJECT and USER layers.
// StandardRootsWithOptions 构造一条可选 PROJECT 与 USER 层的正式 root 链。
func StandardRootsWithOptions(runtimeRoot string, includeProject bool, includeUser bool) []RuntimeSkillRoot {
	root := normalizePath(runtimeRoot)
	roots := []RuntimeSkillRoot{{
		Name:      "ROOT",
		SkillsDir: normalizePath(filepath.Join(root, "root_skills")),
	}}
	if includeProject {
		roots = append(roots, RuntimeSkillRoot{
			Name:      "PROJECT",
			SkillsDir: normalizePath(filepath.Join(root, "project_skills")),
		})
	}
	if includeUser {
		roots = append(roots, RuntimeSkillRoot{
			Name:      "USER",
			SkillsDir: normalizePath(filepath.Join(root, "user_skills")),
		})
	}
	return roots
}

// RootOnly builds one ROOT-only chain for system-only hosts.
// RootOnly 为仅系统层宿主构造一条仅 ROOT 的 root 链。
func RootOnly(runtimeRoot string) []RuntimeSkillRoot {
	return StandardRootsWithOptions(runtimeRoot, false, false)
}

// FindRootByLabel locates one root by formal label using trim and uppercase normalization.
// FindRootByLabel 使用 trim 与大写归一化按正式标签定位单个 root。
func FindRootByLabel(skillRoots []RuntimeSkillRoot, label string) (*RuntimeSkillRoot, error) {
	normalizedLabel := strings.ToUpper(strings.TrimSpace(label))
	for index := range skillRoots {
		if strings.ToUpper(strings.TrimSpace(skillRoots[index].Name)) == normalizedLabel {
			return &skillRoots[index], nil
		}
	}
	return nil, fmt.Errorf("runtime skill root %q is not present in the configured root chain", normalizedLabel)
}

// EnsureRuntimeLayout creates runtime directories needed by default SDK host options and root chain.
// EnsureRuntimeLayout 创建默认 SDK 宿主选项和 root 链所需的运行时目录。
func EnsureRuntimeLayout(runtimeRoot string, skillRoots []RuntimeSkillRoot) error {
	root := normalizePath(runtimeRoot)
	if len(skillRoots) == 0 {
		skillRoots = StandardRoots(root)
	}
	directories := []string{
		root,
		filepath.Join(root, "temp"),
		filepath.Join(root, "temp", "downloads"),
		filepath.Join(root, "resources"),
		filepath.Join(root, "lua_packages"),
		filepath.Join(root, "bin", "tools"),
		filepath.Join(root, "libs"),
		filepath.Join(root, "dependencies"),
		filepath.Join(root, "state"),
		filepath.Join(root, "databases"),
	}
	for _, skillRoot := range skillRoots {
		directories = append(directories, filepath.FromSlash(skillRoot.SkillsDir))
	}
	for _, directory := range directories {
		if err := os.MkdirAll(directory, 0o755); err != nil {
			return err
		}
	}
	return nil
}

// normalizePath returns one absolute slash-normalized filesystem path.
// normalizePath 返回单个绝对且斜杠归一化的文件系统路径。
func normalizePath(path string) string {
	absolute, err := filepath.Abs(path)
	if err != nil {
		return filepath.ToSlash(path)
	}
	return filepath.ToSlash(absolute)
}
