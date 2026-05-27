use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const OUTPUT_DIR: &str = "scripts/output";

fn workspace_root() -> io::Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors() {
            if ancestor.join("scripts").is_dir() && ancestor.join("Cargo.toml").is_file() {
                return Ok(ancestor.to_path_buf());
            }
        }
    }

    std::env::current_dir()
}

pub fn ensure_output_dir() -> io::Result<PathBuf> {
    let dir = workspace_root()?.join(OUTPUT_DIR);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn sanitize_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "default".to_string()
    } else {
        sanitized
    }
}

pub fn dataset_label_from_path(path: &Path) -> String {
    let candidate = if path.extension().is_some() {
        path.parent()
            .and_then(|parent| parent.file_name())
            .or_else(|| path.file_stem())
    } else {
        path.file_name().or_else(|| path.file_stem())
    };

    candidate
        .and_then(|s| s.to_str())
        .map(sanitize_component)
        .unwrap_or_else(|| "default".to_string())
}

pub fn write_scoped_timestamped_report(
    scope_parts: &[&str],
    prefix: &str,
    contents: &str,
) -> io::Result<PathBuf> {
    let mut dir = ensure_output_dir()?;
    for part in scope_parts {
        dir.push(sanitize_component(part));
    }
    fs::create_dir_all(&dir)?;

    let file_name = format!("{}-{}.txt", prefix, timestamp_token());
    let path = dir.join(file_name);
    fs::write(&path, contents)?;
    Ok(path)
}

pub fn write_scoped_report_file(
    scope_parts: &[&str],
    file_name: &str,
    contents: &str,
) -> io::Result<PathBuf> {
    let mut dir = ensure_output_dir()?;
    for part in scope_parts {
        dir.push(sanitize_component(part));
    }
    fs::create_dir_all(&dir)?;

    let path = dir.join(file_name);
    fs::write(&path, contents)?;
    Ok(path)
}

pub fn timestamp_token() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}_{:03}", now.as_secs(), now.subsec_millis())
}

pub fn write_report_file(file_name: &str, contents: &str) -> io::Result<PathBuf> {
    let dir = ensure_output_dir()?;
    let path = dir.join(file_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, contents)?;
    Ok(path)
}

pub fn write_timestamped_report(prefix: &str, contents: &str) -> io::Result<PathBuf> {
    let file_name = format!("{}-{}.txt", prefix, timestamp_token());
    write_report_file(&file_name, contents)
}

pub fn directory_size_bytes(path: &Path) -> io::Result<u64> {
    let mut total = 0u64;
    if !path.exists() {
        return Ok(0);
    }

    let metadata = fs::metadata(path)?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        total += directory_size_bytes(&entry.path())?;
    }

    Ok(total)
}
