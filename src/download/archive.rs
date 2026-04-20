use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;
use zip::ZipArchive;

use crate::skill::dependencies::{DependencyArchiveType, DependencyExportSpec};

/// Install one downloaded payload into the dependency root according to export rules.
/// 按导出规则把单个已下载载荷安装到依赖根目录。
pub fn install_downloaded_payload(
    archive_path: &Path,
    archive_type: DependencyArchiveType,
    install_root: &Path,
    exports: &[DependencyExportSpec],
) -> Result<(), String> {
    fs::create_dir_all(install_root)
        .map_err(|error| format!("Failed to create {}: {}", install_root.display(), error))?;
    match archive_type {
        DependencyArchiveType::Raw => install_from_raw_file(archive_path, install_root, exports),
        DependencyArchiveType::Zip => install_from_zip_archive(archive_path, install_root, exports),
        DependencyArchiveType::TarGz => {
            install_from_tar_gz_archive(archive_path, install_root, exports)
        }
    }
}

/// Extract one skill package zip into a temporary root and return the extracted skill directory.
/// 把单个技能包 zip 解压到临时根目录，并返回解压得到的技能目录。
pub fn extract_skill_package_zip(
    archive_path: &Path,
    temp_root: &Path,
    expected_skill_id: &str,
) -> Result<PathBuf, String> {
    fs::create_dir_all(temp_root)
        .map_err(|error| format!("Failed to create {}: {}", temp_root.display(), error))?;
    let file = fs::File::open(archive_path)
        .map_err(|error| format!("Failed to open {}: {}", archive_path.display(), error))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("Failed to open zip archive: {}", error))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Failed to read zip entry #{}: {}", index, error))?;
        let entry_path = normalize_zip_entry_path(entry.name())?;
        if entry_path.components().next().is_none() {
            continue;
        }

        let top_level = entry_path
            .components()
            .next()
            .and_then(|component| component.as_os_str().to_str())
            .ok_or_else(|| {
                format!(
                    "Failed to read the top-level directory of zip entry '{}'",
                    entry.name()
                )
            })?;
        if top_level != expected_skill_id {
            return Err(format!(
                "Skill package {} must contain only the top-level directory '{}', but found '{}'",
                archive_path.display(),
                expected_skill_id,
                top_level
            ));
        }

        let target_path = temp_root.join(&entry_path);
        if entry.is_dir() {
            fs::create_dir_all(&target_path).map_err(|error| {
                format!("Failed to create {}: {}", target_path.display(), error)
            })?;
            continue;
        }

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;
        }
        let mut output = fs::File::create(&target_path)
            .map_err(|error| format!("Failed to create {}: {}", target_path.display(), error))?;
        std::io::copy(&mut entry, &mut output).map_err(|error| {
            format!(
                "Failed to extract '{}' into {}: {}",
                entry.name(),
                target_path.display(),
                error
            )
        })?;
    }

    let skill_dir = temp_root.join(expected_skill_id);
    let skill_yaml = skill_dir.join("skill.yaml");
    if !skill_yaml.exists() {
        return Err(format!(
            "Skill package {} does not contain {}/skill.yaml",
            archive_path.display(),
            expected_skill_id
        ));
    }
    Ok(skill_dir)
}

/// Install exports from one raw single-file payload.
/// 从单个原始文件载荷中安装导出文件。
fn install_from_raw_file(
    archive_path: &Path,
    install_root: &Path,
    exports: &[DependencyExportSpec],
) -> Result<(), String> {
    if exports.len() != 1 {
        return Err("raw dependency payload must declare exactly one export".to_string());
    }
    let export = &exports[0];
    let target_path = join_relative_target(install_root, &export.target_path);
    copy_file_with_parent_dir(archive_path, &target_path)?;
    mark_executable_if_needed(&target_path, export.executable)?;
    Ok(())
}

/// Install exports from one zip archive payload.
/// 从单个 zip 归档载荷中安装导出文件。
fn install_from_zip_archive(
    archive_path: &Path,
    install_root: &Path,
    exports: &[DependencyExportSpec],
) -> Result<(), String> {
    let file = fs::File::open(archive_path)
        .map_err(|error| format!("Failed to open {}: {}", archive_path.display(), error))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("Failed to open zip archive: {}", error))?;
    for export in exports {
        let entry_name = resolve_zip_export_entry_name(&mut archive, &export.archive_path)
            .ok_or_else(|| {
                format!(
                    "Failed to read zip entry '{}' from {}: specified file not found in archive",
                    export.archive_path,
                    archive_path.display()
                )
            })?;
        let mut entry = archive.by_name(&entry_name).map_err(|error| {
            format!(
                "Failed to read zip entry '{}' from {}: {}",
                entry_name,
                archive_path.display(),
                error
            )
        })?;
        let target_path = join_relative_target(install_root, &export.target_path);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;
        }
        let mut output = fs::File::create(&target_path)
            .map_err(|error| format!("Failed to create {}: {}", target_path.display(), error))?;
        std::io::copy(&mut entry, &mut output).map_err(|error| {
            format!(
                "Failed to extract '{}' into {}: {}",
                export.archive_path,
                target_path.display(),
                error
            )
        })?;
        mark_executable_if_needed(&target_path, export.executable)?;
    }
    Ok(())
}

/// Install exports from one tar.gz archive payload.
/// 从单个 tar.gz 归档载荷中安装导出文件。
fn install_from_tar_gz_archive(
    archive_path: &Path,
    install_root: &Path,
    exports: &[DependencyExportSpec],
) -> Result<(), String> {
    let bytes = fs::read(archive_path)
        .map_err(|error| format!("Failed to read {}: {}", archive_path.display(), error))?;
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(decoder);
    let mut extracted_entries: Vec<(PathBuf, bool)> = Vec::new();

    for archive_entry in archive.entries().map_err(|error| {
        format!(
            "Failed to enumerate tar.gz entries from {}: {}",
            archive_path.display(),
            error
        )
    })? {
        let mut archive_entry =
            archive_entry.map_err(|error| format!("Failed to read tar entry: {}", error))?;
        let entry_path = archive_entry
            .path()
            .map_err(|error| format!("Failed to read tar entry path: {}", error))?
            .to_string_lossy()
            .replace('\\', "/");
        if let Some(export) = exports
            .iter()
            .find(|export| archive_entry_matches_export(&entry_path, &export.archive_path))
        {
            let target_path = join_relative_target(install_root, &export.target_path);
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;
            }
            let mut output = fs::File::create(&target_path).map_err(|error| {
                format!("Failed to create {}: {}", target_path.display(), error)
            })?;
            let mut buffer = Vec::new();
            archive_entry.read_to_end(&mut buffer).map_err(|error| {
                format!(
                    "Failed to extract '{}' from {}: {}",
                    export.archive_path,
                    archive_path.display(),
                    error
                )
            })?;
            std::io::copy(&mut Cursor::new(buffer), &mut output)
                .map_err(|error| format!("Failed to write {}: {}", target_path.display(), error))?;
            extracted_entries.push((target_path, export.executable));
        }
    }

    for export in exports {
        let target_path = join_relative_target(install_root, &export.target_path);
        if !target_path.exists() {
            return Err(format!(
                "tar.gz archive {} does not contain required export '{}'",
                archive_path.display(),
                export.archive_path
            ));
        }
    }

    for (target_path, executable) in extracted_entries {
        mark_executable_if_needed(&target_path, executable)?;
    }
    Ok(())
}

/// Resolve one zip export entry by exact path first and then by stripping one leading archive directory.
/// 先按精确路径解析单个 zip 导出条目，再尝试剥离一层归档顶层目录进行匹配。
fn resolve_zip_export_entry_name<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    expected_archive_path: &str,
) -> Option<String> {
    let expected = normalize_archive_entry_match_path(expected_archive_path);
    if archive
        .file_names()
        .any(|name| normalize_archive_entry_match_path(name) == expected)
    {
        return Some(expected);
    }

    archive.file_names().find_map(|name| {
        let normalized_name = normalize_archive_entry_match_path(name);
        if strip_one_leading_archive_component(&normalized_name).as_deref()
            == Some(expected.as_str())
        {
            Some(normalized_name)
        } else {
            None
        }
    })
}

/// Return whether one archive entry matches one declared export path directly or after stripping one top-level directory.
/// 判断单个归档条目是否能与声明的导出路径直接匹配，或在剥离一层顶层目录后匹配。
fn archive_entry_matches_export(entry_path: &str, export_archive_path: &str) -> bool {
    let normalized_entry = normalize_archive_entry_match_path(entry_path);
    let normalized_export = normalize_archive_entry_match_path(export_archive_path);
    normalized_entry == normalized_export
        || strip_one_leading_archive_component(&normalized_entry).as_deref()
            == Some(normalized_export.as_str())
}

/// Normalize one archive entry path into a stable forward-slash matching representation.
/// 把单个归档条目路径规范化为稳定的正斜杠匹配表示。
fn normalize_archive_entry_match_path(raw: &str) -> String {
    raw.replace('\\', "/").trim_matches('/').to_string()
}

/// Strip exactly one leading path component from one normalized archive entry path.
/// 从一个已规范化的归档条目路径中剥离恰好一层顶层路径片段。
fn strip_one_leading_archive_component(normalized_path: &str) -> Option<String> {
    let mut components = normalized_path
        .split('/')
        .filter(|component| !component.is_empty());
    components.next()?;
    let remainder = components.collect::<Vec<_>>();
    if remainder.is_empty() {
        None
    } else {
        Some(remainder.join("/"))
    }
}

/// Join one relative target path under the dependency root.
/// 把单个相对目标路径拼接到依赖根目录下。
fn join_relative_target(root: &Path, relative_target: &str) -> PathBuf {
    let normalized = relative_target.replace('/', std::path::MAIN_SEPARATOR_STR);
    root.join(normalized)
}

/// Copy one file into the target path and create parent directories first.
/// 将单个文件复制到目标路径，并在复制前创建父级目录。
fn copy_file_with_parent_dir(source: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;
    }
    fs::copy(source, target).map_err(|error| {
        format!(
            "Failed to copy {} to {}: {}",
            source.display(),
            target.display(),
            error
        )
    })?;
    Ok(())
}

/// Mark one target file executable on Unix platforms when requested.
/// 在需要时把单个目标文件在 Unix 平台上标记为可执行。
fn mark_executable_if_needed(_target: &Path, executable: bool) -> Result<(), String> {
    if !executable {
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(_target)
            .map_err(|error| format!("Failed to stat {}: {}", _target.display(), error))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(_target, permissions)
            .map_err(|error| format!("Failed to chmod {}: {}", _target.display(), error))?;
    }

    Ok(())
}

/// Normalize one zip entry path and reject traversal or absolute-path entries.
/// 规范化单个 zip 条目路径，并拒绝目录穿越或绝对路径条目。
fn normalize_zip_entry_path(entry_name: &str) -> Result<PathBuf, String> {
    let normalized = entry_name.replace('\\', "/");
    let mut path = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            std::path::Component::Normal(value) => path.push(value),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                return Err(format!(
                    "Zip entry '{}' must not contain parent-directory traversal",
                    entry_name
                ));
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "Zip entry '{}' must not use an absolute path",
                    entry_name
                ));
            }
        }
    }
    Ok(path)
}
