use super::*;
use crate::skill::dependencies::{
    DependencyArchiveType, DependencyExportSpec, DependencyPackageSpec, DependencySourceSpec,
    GithubReleaseSourceSpec, SkillDependencyManifest, ToolDependencySpec, UrlSourceSpec,
};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Build one minimal dependency manager rooted under one unique temporary test directory.
/// 在唯一的临时测试目录下构造一个最小依赖管理器。
fn test_manager() -> (DependencyManager, PathBuf) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("luaskills-dependency-test-{}", unique));
    let config = DependencyManagerConfig {
        tool_root: root.join("dependencies").join("tools"),
        host_tool_root: root.join("bin").join("tools"),
        lua_root: root.join("dependencies").join("lua"),
        host_lua_root: root.join("lua_packages"),
        ffi_root: root.join("dependencies").join("ffi"),
        host_ffi_root: root.join("libs"),
        download_cache_root: root.join("temp").join("downloads"),
        allow_network_download: false,
        github_base_url: None,
        github_api_base_url: None,
    };
    (DependencyManager::new(config), root)
}

/// Build one minimal tool dependency spec for the current platform.
/// 为当前平台构造一个最小工具依赖声明。
fn tool_dependency(name: &str, version: &str, platform_key: &str) -> ToolDependencySpec {
    let mut packages = BTreeMap::new();
    packages.insert(
        platform_key.to_string(),
        DependencyPackageSpec {
            archive_type: DependencyArchiveType::Raw,
            asset_name: None,
            url: Some("https://example.invalid/package".to_string()),
            exports: vec![DependencyExportSpec {
                archive_path: "demo.bin".to_string(),
                target_path: "bin/demo.bin".to_string(),
                executable: false,
            }],
        },
    );
    ToolDependencySpec {
        name: name.to_string(),
        version: Some(version.to_string()),
        required: true,
        scope: DependencyScope::Skill,
        source: DependencySourceSpec {
            source_type: DependencySourceType::Url,
            github: None,
            url: Some(UrlSourceSpec::default()),
            skilllist: None,
        },
        packages,
    }
}

/// Build one GitHub-release tool dependency that must not touch GitHub when exports exist.
/// 构造一个在导出产物已存在时不应访问 GitHub 的 GitHub Release 工具依赖。
fn github_tool_dependency(
    name: &str,
    version: Option<&str>,
    platform_key: &str,
) -> ToolDependencySpec {
    let mut packages = BTreeMap::new();
    packages.insert(
        platform_key.to_string(),
        DependencyPackageSpec {
            archive_type: DependencyArchiveType::Zip,
            asset_name: Some("demo-{version}.zip".to_string()),
            url: None,
            exports: vec![DependencyExportSpec {
                archive_path: "demo-{version}.bin".to_string(),
                target_path: "bin/demo-{version}.bin".to_string(),
                executable: false,
            }],
        },
    );
    ToolDependencySpec {
        name: name.to_string(),
        version: version.map(str::to_string),
        required: true,
        scope: DependencyScope::Skill,
        source: DependencySourceSpec {
            source_type: DependencySourceType::GithubRelease,
            github: Some(GithubReleaseSourceSpec {
                repo: "OpenVulcan/demo-dependency".to_string(),
                tag_api: None,
            }),
            url: None,
            skilllist: None,
        },
        packages,
    }
}

/// Build one GitHub-release tool dependency whose export target uses the release tag.
/// 构造一个导出目标使用 release 标签的 GitHub Release 工具依赖。
fn github_tagged_tool_dependency(
    name: &str,
    version: Option<&str>,
    platform_key: &str,
) -> ToolDependencySpec {
    let mut dependency = github_tool_dependency(name, version, platform_key);
    let package = dependency
        .packages
        .get_mut(platform_key)
        .expect("test package should exist");
    package.exports = vec![DependencyExportSpec {
        archive_path: "demo-{tag}.bin".to_string(),
        target_path: "bin/demo-{tag}.bin".to_string(),
        executable: false,
    }];
    dependency
}

/// Existing GitHub-release exports should enable the skill without remote resolution.
/// 已存在的 GitHub Release 导出产物应直接启用 skill 而不进行远程解析。
#[test]
fn ensure_dependency_reuses_existing_github_release_exports_without_remote_resolution() {
    let platform_key = current_platform_key();
    if platform_key == "unknown" {
        return;
    }

    let (mut manager, root) = test_manager();
    manager.config.allow_network_download = true;
    manager.config.github_api_base_url = Some("https://example.invalid/github-api".to_string());
    manager.downloader = DownloadManager::new(DownloadManagerConfig {
        cache_root: manager.config.download_cache_root.clone(),
        allow_network_download: manager.config.allow_network_download,
        github_base_url: manager.config.github_base_url.clone(),
        github_api_base_url: manager.config.github_api_base_url.clone(),
    });
    let skill_id = "demo-skill";
    let manifest = SkillDependencyManifest {
        tool_dependencies: vec![github_tool_dependency(
            "demo-tool",
            Some("1.2.3"),
            platform_key,
        )],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };
    let dependency_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "demo-tool",
        Some("1.2.3"),
        platform_key,
    );
    fs::create_dir_all(dependency_root.join("bin")).unwrap();
    fs::write(dependency_root.join("bin").join("demo-1.2.3.bin"), b"ready").unwrap();

    manager
        .ensure_skill_dependencies(skill_id, &manifest)
        .expect("existing exports should bypass GitHub release lookup");

    let _ = fs::remove_dir_all(root);
}

/// Existing version directories should be reused when a GitHub dependency omits version.
/// 当 GitHub 依赖省略版本时，应复用已有版本目录中的导出产物。
#[test]
fn ensure_dependency_reuses_existing_unversioned_github_release_exports() {
    let platform_key = current_platform_key();
    if platform_key == "unknown" {
        return;
    }

    let (mut manager, root) = test_manager();
    manager.config.allow_network_download = true;
    manager.config.github_api_base_url = Some("https://example.invalid/github-api".to_string());
    manager.downloader = DownloadManager::new(DownloadManagerConfig {
        cache_root: manager.config.download_cache_root.clone(),
        allow_network_download: manager.config.allow_network_download,
        github_base_url: manager.config.github_base_url.clone(),
        github_api_base_url: manager.config.github_api_base_url.clone(),
    });
    let skill_id = "demo-skill";
    let manifest = SkillDependencyManifest {
        tool_dependencies: vec![github_tool_dependency("demo-tool", None, platform_key)],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };
    let dependency_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "demo-tool",
        Some("1.2.3"),
        platform_key,
    );
    fs::create_dir_all(dependency_root.join("bin")).unwrap();
    fs::write(dependency_root.join("bin").join("demo-1.2.3.bin"), b"ready").unwrap();

    manager
        .ensure_skill_dependencies(skill_id, &manifest)
        .expect("existing version directories should bypass GitHub release lookup");

    let _ = fs::remove_dir_all(root);
}

/// Existing GitHub-release exports using `{tag}` should reuse the likely `v` tag variant.
/// 使用 `{tag}` 的 GitHub Release 已有导出产物应复用可能的 `v` 标签变体。
#[test]
fn ensure_dependency_reuses_existing_github_release_tag_exports_without_remote_resolution() {
    let platform_key = current_platform_key();
    if platform_key == "unknown" {
        return;
    }

    let (mut manager, root) = test_manager();
    manager.config.allow_network_download = true;
    manager.config.github_api_base_url = Some("https://example.invalid/github-api".to_string());
    manager.downloader = DownloadManager::new(DownloadManagerConfig {
        cache_root: manager.config.download_cache_root.clone(),
        allow_network_download: manager.config.allow_network_download,
        github_base_url: manager.config.github_base_url.clone(),
        github_api_base_url: manager.config.github_api_base_url.clone(),
    });
    let skill_id = "demo-skill";
    let manifest = SkillDependencyManifest {
        tool_dependencies: vec![github_tagged_tool_dependency(
            "demo-tool",
            Some("1.2.3"),
            platform_key,
        )],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };
    let dependency_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "demo-tool",
        Some("1.2.3"),
        platform_key,
    );
    fs::create_dir_all(dependency_root.join("bin")).unwrap();
    fs::write(
        dependency_root.join("bin").join("demo-v1.2.3.bin"),
        b"ready",
    )
    .unwrap();

    manager
        .ensure_skill_dependencies(skill_id, &manifest)
        .expect("existing tag exports should bypass GitHub release lookup");

    let _ = fs::remove_dir_all(root);
}

/// Existing unversioned GitHub exports using `{tag}` should reuse scanned version roots.
/// 未声明版本且使用 `{tag}` 的 GitHub 已有导出产物应复用扫描到的版本根目录。
#[test]
fn ensure_dependency_reuses_existing_unversioned_github_release_tag_exports() {
    let platform_key = current_platform_key();
    if platform_key == "unknown" {
        return;
    }

    let (mut manager, root) = test_manager();
    manager.config.allow_network_download = true;
    manager.config.github_api_base_url = Some("https://example.invalid/github-api".to_string());
    manager.downloader = DownloadManager::new(DownloadManagerConfig {
        cache_root: manager.config.download_cache_root.clone(),
        allow_network_download: manager.config.allow_network_download,
        github_base_url: manager.config.github_base_url.clone(),
        github_api_base_url: manager.config.github_api_base_url.clone(),
    });
    let skill_id = "demo-skill";
    let manifest = SkillDependencyManifest {
        tool_dependencies: vec![github_tagged_tool_dependency(
            "demo-tool",
            None,
            platform_key,
        )],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };
    let dependency_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "demo-tool",
        Some("1.2.3"),
        platform_key,
    );
    fs::create_dir_all(dependency_root.join("bin")).unwrap();
    fs::write(
        dependency_root.join("bin").join("demo-v1.2.3.bin"),
        b"ready",
    )
    .unwrap();

    manager
        .ensure_skill_dependencies(skill_id, &manifest)
        .expect("existing unversioned tag exports should bypass GitHub release lookup");

    let _ = fs::remove_dir_all(root);
}

/// Updated-skill cleanup removes stale private dependency roots while preserving unchanged ones.
/// 更新后的清理流程会删除过期的私有依赖根，同时保留未变化的依赖。
#[test]
fn cleanup_updated_skill_dependencies_removes_stale_roots_and_keeps_reused_roots() {
    let platform_key = current_platform_key();
    if platform_key == "unknown" {
        return;
    }

    let (manager, root) = test_manager();
    let skill_id = "demo-skill";
    let previous_manifest = SkillDependencyManifest {
        tool_dependencies: vec![
            tool_dependency("rg", "14.1.1", platform_key),
            tool_dependency("fd", "9.0.0", platform_key),
        ],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };
    let current_manifest = SkillDependencyManifest {
        tool_dependencies: vec![
            tool_dependency("rg", "14.1.2", platform_key),
            tool_dependency("fd", "9.0.0", platform_key),
        ],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };

    let stale_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "rg",
        Some("14.1.1"),
        platform_key,
    );
    let kept_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "fd",
        Some("9.0.0"),
        platform_key,
    );
    let current_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "rg",
        Some("14.1.2"),
        platform_key,
    );

    fs::create_dir_all(stale_root.join("bin")).unwrap();
    fs::write(stale_root.join("bin").join("demo.bin"), b"old").unwrap();
    fs::create_dir_all(kept_root.join("bin")).unwrap();
    fs::write(kept_root.join("bin").join("demo.bin"), b"keep").unwrap();
    fs::create_dir_all(current_root.join("bin")).unwrap();
    fs::write(current_root.join("bin").join("demo.bin"), b"new").unwrap();

    manager
        .cleanup_updated_skill_dependencies(
            skill_id,
            Some(&previous_manifest),
            Some(&current_manifest),
        )
        .unwrap();

    assert!(
        !stale_root.exists(),
        "stale dependency root should be removed"
    );
    assert!(
        kept_root.exists(),
        "unchanged dependency root should be preserved"
    );
    assert!(
        current_root.exists(),
        "current dependency root should be preserved"
    );

    let _ = fs::remove_dir_all(root);
}

/// Updated-skill cleanup keeps identical dependency roots when the manifest does not change.
/// 当依赖清单没有变化时，更新清理流程会保留完全相同的依赖根目录。
#[test]
fn cleanup_updated_skill_dependencies_keeps_identical_roots() {
    let platform_key = current_platform_key();
    if platform_key == "unknown" {
        return;
    }

    let (manager, root) = test_manager();
    let skill_id = "demo-skill";
    let manifest = SkillDependencyManifest {
        tool_dependencies: vec![tool_dependency("rg", "14.1.1", platform_key)],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };

    let dependency_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "rg",
        Some("14.1.1"),
        platform_key,
    );
    fs::create_dir_all(dependency_root.join("bin")).unwrap();
    fs::write(dependency_root.join("bin").join("demo.bin"), b"keep").unwrap();

    manager
        .cleanup_updated_skill_dependencies(skill_id, Some(&manifest), Some(&manifest))
        .unwrap();

    assert!(
        dependency_root.exists(),
        "unchanged dependency root should remain"
    );

    let _ = fs::remove_dir_all(root);
}

/// Updated-skill cleanup removes all old private dependency roots that disappear from the new manifest.
/// 当新清单移除了旧依赖时，更新清理流程会删除全部过期的私有依赖根目录。
#[test]
fn cleanup_updated_skill_dependencies_removes_deleted_dependencies() {
    let platform_key = current_platform_key();
    if platform_key == "unknown" {
        return;
    }

    let (manager, root) = test_manager();
    let skill_id = "demo-skill";
    let previous_manifest = SkillDependencyManifest {
        tool_dependencies: vec![
            tool_dependency("rg", "14.1.1", platform_key),
            tool_dependency("fd", "9.0.0", platform_key),
        ],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };
    let current_manifest = SkillDependencyManifest::default();

    let rg_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "rg",
        Some("14.1.1"),
        platform_key,
    );
    let fd_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "fd",
        Some("9.0.0"),
        platform_key,
    );
    fs::create_dir_all(rg_root.join("bin")).unwrap();
    fs::write(rg_root.join("bin").join("demo.bin"), b"old-rg").unwrap();
    fs::create_dir_all(fd_root.join("bin")).unwrap();
    fs::write(fd_root.join("bin").join("demo.bin"), b"old-fd").unwrap();

    manager
        .cleanup_updated_skill_dependencies(
            skill_id,
            Some(&previous_manifest),
            Some(&current_manifest),
        )
        .unwrap();

    assert!(
        !rg_root.exists(),
        "removed dependency root should be deleted"
    );
    assert!(
        !fd_root.exists(),
        "removed dependency root should be deleted"
    );

    let _ = fs::remove_dir_all(root);
}

/// Updated-skill cleanup does not remove current roots when the new manifest only adds dependencies.
/// 当新清单只是新增依赖时，更新清理流程不会误删当前仍然有效的依赖根目录。
#[test]
fn cleanup_updated_skill_dependencies_preserves_existing_roots_for_add_only_changes() {
    let platform_key = current_platform_key();
    if platform_key == "unknown" {
        return;
    }

    let (manager, root) = test_manager();
    let skill_id = "demo-skill";
    let previous_manifest = SkillDependencyManifest {
        tool_dependencies: vec![tool_dependency("rg", "14.1.1", platform_key)],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };
    let current_manifest = SkillDependencyManifest {
        tool_dependencies: vec![
            tool_dependency("rg", "14.1.1", platform_key),
            tool_dependency("fd", "9.0.0", platform_key),
        ],
        lua_dependencies: Vec::new(),
        ffi_dependencies: Vec::new(),
    };

    let rg_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "rg",
        Some("14.1.1"),
        platform_key,
    );
    let fd_root = build_dependency_install_root(
        &manager.config.tool_root,
        DependencyScope::Skill,
        skill_id,
        "fd",
        Some("9.0.0"),
        platform_key,
    );
    fs::create_dir_all(rg_root.join("bin")).unwrap();
    fs::write(rg_root.join("bin").join("demo.bin"), b"keep-rg").unwrap();
    fs::create_dir_all(fd_root.join("bin")).unwrap();
    fs::write(fd_root.join("bin").join("demo.bin"), b"new-fd").unwrap();

    manager
        .cleanup_updated_skill_dependencies(
            skill_id,
            Some(&previous_manifest),
            Some(&current_manifest),
        )
        .unwrap();

    assert!(
        rg_root.exists(),
        "existing dependency root should be preserved"
    );
    assert!(
        fd_root.exists(),
        "new dependency root should remain untouched"
    );

    let _ = fs::remove_dir_all(root);
}

/// GitHub-release export templates should resolve `{version}` placeholders before archive extraction.
/// GitHub Release 导出模板应在归档解包前解析 `{version}` 占位符。
#[test]
fn resolve_export_templates_expands_version_placeholder() {
    let exports = vec![DependencyExportSpec {
        archive_path: "ripgrep-{version}-x86_64-pc-windows-msvc/rg.exe".to_string(),
        target_path: "bin/rg-{version}.exe".to_string(),
        executable: false,
    }];

    let resolved = resolve_export_templates(&exports, Some("14.1.1"), Some("14.1.1"));
    assert_eq!(
        resolved[0].archive_path,
        "ripgrep-14.1.1-x86_64-pc-windows-msvc/rg.exe"
    );
    assert_eq!(resolved[0].target_path, "bin/rg-14.1.1.exe");
}
