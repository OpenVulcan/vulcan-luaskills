use super::{
    SkillInstallRequest, SkillInstallSourceType, SkillManager, SkillManagerConfig,
    SkillOperationPlane, TempDirGuard, collect_effective_skill_instances,
    resolve_effective_skill_instance,
};
use crate::runtime_options::RuntimeSkillRoot;

/// Build one test skill-manager configuration rooted under the provided temporary directory.
/// 基于给定临时目录构造单个测试用技能管理器配置。
fn test_manager_config(
    temp_root: &std::path::Path,
    skill_root: RuntimeSkillRoot,
) -> SkillManagerConfig {
    SkillManagerConfig {
        skill_root,
        lifecycle_root: temp_root.join("state"),
        download_cache_root: temp_root.join("downloads"),
        allow_network_download: false,
        github_base_url: None,
        github_api_base_url: None,
    }
}

/// Verify that the staging-directory guard cleans temp roots on drop.
/// 验证暂存目录守卫会在析构时清理临时根目录。
#[test]
fn temp_dir_guard_removes_staging_root_on_drop() {
    let temp_root =
        std::env::temp_dir().join(format!("luaskills_temp_guard_test_{}", std::process::id()));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }
    std::fs::create_dir_all(&temp_root).expect("temp root should be created");
    {
        let _guard = TempDirGuard::new(temp_root.clone());
        std::fs::write(temp_root.join("staged.txt"), "staged")
            .expect("staged marker should be written");
    }
    assert!(!temp_root.exists());
}

/// Verify that disable/enable operations persist and clear state markers correctly.
/// 验证停用/启用操作会正确持久化并清理状态标记。
#[test]
fn skill_manager_persists_disabled_state() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_skill_manager_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }
    let skill_root = temp_root.join("skills");
    let manager = SkillManager::new(SkillManagerConfig {
        ..test_manager_config(
            &temp_root,
            RuntimeSkillRoot {
                name: "USER".to_string(),
                skills_dir: skill_root,
            },
        )
    });

    assert!(manager.is_skill_enabled("vulcan-codekit").unwrap());
    manager
        .disable_skill("vulcan-codekit", Some("manual test"))
        .expect("disable should succeed");
    assert!(!manager.is_skill_enabled("vulcan-codekit").unwrap());
    assert_eq!(
        manager
            .disabled_record("vulcan-codekit")
            .unwrap()
            .expect("record should exist")
            .reason
            .as_deref(),
        Some("manual test")
    );

    manager
        .enable_skill("vulcan-codekit")
        .expect("enable should succeed");
    assert!(manager.is_skill_enabled("vulcan-codekit").unwrap());

    let _ = std::fs::remove_dir_all(&temp_root);
}

/// Verify that install/update entrypoints return strict structured states before networking succeeds.
/// 验证 install/update 入口在真正下载前会返回严格的结构化状态。
#[test]
fn install_update_entrypoints_return_strict_structured_results() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_install_update_test_{}",
        std::process::id()
    ));
    let skill_root = temp_root.join("skills");
    let skill_roots = vec![RuntimeSkillRoot {
        name: "USER".to_string(),
        skills_dir: skill_root.clone(),
    }];
    let _ = std::fs::create_dir_all(&skill_root);
    let manager = SkillManager::new(test_manager_config(&temp_root, skill_roots[0].clone()));

    let install_result = manager
        .prepare_install_skill(
            SkillOperationPlane::Skills,
            &skill_roots,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("install without source should fail strictly");
    assert!(install_result.contains("github install requires source repository"));

    let _ = std::fs::create_dir_all(skill_root.join("vulcan-codekit"));
    let update_result = manager
        .prepare_update_skill(
            SkillOperationPlane::Skills,
            &skill_roots,
            &SkillInstallRequest {
                skill_id: Some("vulcan-codekit".to_string()),
                source: None,
                source_type: SkillInstallSourceType::Github,
            },
        )
        .expect_err("update without install record should fail strictly");
    assert!(update_result.contains("is not managed by the install workflow"));

    let _ = std::fs::remove_dir_all(&temp_root);
}

/// Verify that uninstall removes the skill directory but keeps database flags unset by default.
/// 验证卸载会删除技能目录，同时默认不声明数据库已删除。
#[test]
fn uninstall_returns_safe_default_database_flags() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_uninstall_result_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }
    let skill_root = temp_root.join("skills");
    let manager = SkillManager::new(test_manager_config(
        &temp_root,
        RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: skill_root.clone(),
        },
    ));
    let _ = std::fs::create_dir_all(skill_root.join("vulcan-codekit"));

    let result = manager
        .uninstall_skill("vulcan-codekit")
        .expect("uninstall should succeed");
    assert!(result.skill_removed);
    assert!(!result.sqlite_removed);
    assert!(!result.lancedb_removed);
    assert!(!skill_root.join("vulcan-codekit").exists());

    let _ = std::fs::remove_dir_all(&temp_root);
}

/// Verify that PROJECT roots can contribute standalone skills without shadowing ROOT skills.
/// 验证 PROJECT 根目录可以独立提供技能，但不能覆盖 ROOT 技能。
#[test]
fn collect_effective_skill_instances_keeps_root_priority_over_project() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_collect_effective_instances_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }
    let base_dir = temp_root.join("base");
    let override_dir = temp_root.join("override");
    let _ = std::fs::create_dir_all(base_dir.join("vulcan-codekit"));
    let _ = std::fs::create_dir_all(override_dir.join("vulcan-codekit"));
    let _ = std::fs::create_dir_all(override_dir.join("vulcan-runtime"));
    let _ = std::fs::write(
        base_dir.join("vulcan-codekit").join("skill.yaml"),
        "name: vulcan-codekit\nversion: 0.1.0\n",
    );
    let _ = std::fs::write(
        override_dir.join("vulcan-codekit").join("skill.yaml"),
        "name: vulcan-codekit\nversion: 0.2.0\n",
    );
    let _ = std::fs::write(
        override_dir.join("vulcan-runtime").join("skill.yaml"),
        "name: vulcan-runtime\nversion: 0.1.0\n",
    );

    let resolved = collect_effective_skill_instances(&base_dir, Some(&override_dir))
        .expect("effective skill collection should succeed");
    assert_eq!(resolved.len(), 2);
    let codekit = resolved
        .iter()
        .find(|value| value.skill_id == "vulcan-codekit")
        .expect("vulcan-codekit should exist");
    assert!(codekit.actual_dir.starts_with(&base_dir));
    let runtime = resolved
        .iter()
        .find(|value| value.skill_id == "vulcan-runtime")
        .expect("project-only vulcan-runtime should exist");
    assert!(runtime.actual_dir.starts_with(&override_dir));

    let _ = std::fs::remove_dir_all(&temp_root);
}

/// Verify that resolving one effective skill instance keeps ROOT ahead of PROJECT.
/// 验证解析单个生效技能实例时会保持 ROOT 高于 PROJECT。
#[test]
fn resolve_effective_skill_instance_prefers_root_directory() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_resolve_effective_instance_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }
    let base_dir = temp_root.join("base");
    let override_dir = temp_root.join("override");
    let _ = std::fs::create_dir_all(base_dir.join("vulcan-codekit"));
    let _ = std::fs::create_dir_all(override_dir.join("vulcan-codekit"));
    let _ = std::fs::write(
        base_dir.join("vulcan-codekit").join("skill.yaml"),
        "name: vulcan-codekit\nversion: 0.1.0\n",
    );
    let _ = std::fs::write(
        override_dir.join("vulcan-codekit").join("skill.yaml"),
        "name: vulcan-codekit\nversion: 0.2.0\n",
    );

    let resolved =
        resolve_effective_skill_instance(&base_dir, Some(&override_dir), "vulcan-codekit")
            .expect("resolution should succeed")
            .expect("instance should exist");
    assert!(resolved.actual_dir.starts_with(&base_dir));

    let _ = std::fs::remove_dir_all(&temp_root);
}
