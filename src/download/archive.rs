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

/// Install exports from one raw single-file payload.
/// 从单个原始文件载荷中安装导出文件。
fn install_from_raw_file(
    archive_path: &Path,
    install_root: &Path,
    exports: &[DependencyExportSpec],
) -> Result<(), String> {
    if exports.len() != 1 {
        return Err(
            "raw dependency payload must declare exactly one export"
                .to_string(),
        );
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
        let mut entry = archive.by_name(&export.archive_path).map_err(|error| {
            format!("Failed to read zip entry '{}' from {}: {}", export.archive_path, archive_path.display(), error)
        })?;
        let target_path = join_relative_target(install_root, &export.target_path);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;
        }
        let mut output = fs::File::create(&target_path)
            .map_err(|error| format!("Failed to create {}: {}", target_path.display(), error))?;
        std::io::copy(&mut entry, &mut output).map_err(|error| {
            format!("Failed to extract '{}' into {}: {}", export.archive_path, target_path.display(), error)
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
        format!("Failed to enumerate tar.gz entries from {}: {}", archive_path.display(), error)
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
            .find(|export| export.archive_path.replace('\\', "/") == entry_path)
        {
            let target_path = join_relative_target(install_root, &export.target_path);
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!("Failed to create {}: {}", parent.display(), error)
                })?;
            }
            let mut output = fs::File::create(&target_path).map_err(|error| {
                format!("Failed to create {}: {}", target_path.display(), error)
            })?;
            let mut buffer = Vec::new();
            archive_entry.read_to_end(&mut buffer).map_err(|error| {
                format!("Failed to extract '{}' from {}: {}", export.archive_path, archive_path.display(), error)
            })?;
            std::io::copy(&mut Cursor::new(buffer), &mut output).map_err(|error| {
                format!("Failed to write {}: {}", target_path.display(), error)
            })?;
            extracted_entries.push((target_path, export.executable));
        }
    }

    for export in exports {
        let target_path = join_relative_target(install_root, &export.target_path);
        if !target_path.exists() {
            return Err(format!("tar.gz archive {} does not contain required export '{}'", archive_path.display(), export.archive_path));
        }
    }

    for (target_path, executable) in extracted_entries {
        mark_executable_if_needed(&target_path, executable)?;
    }
    Ok(())
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
        format!("Failed to copy {} to {}: {}", source.display(), target.display(), error)
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
