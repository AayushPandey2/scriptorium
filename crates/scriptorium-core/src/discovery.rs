//! Read-only discovery of Obsidian templates; dynamic expressions are reported,
//! never evaluated. All configured paths are checked against the vault boundary.
use crate::validation::safe_path;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn read_json(vault: &Path, relative: &str) -> Result<Option<Value>> {
    let path = safe_path(vault, Path::new(relative), false)?;
    if !path.exists() {
        return Ok(None);
    }
    let path = safe_path(vault, Path::new(relative), true)?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 1024 * 1024,
        "template configuration exceeds 1MiB"
    );
    let bytes = std::fs::read(path)?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    Ok(Some(serde_json::from_slice(bytes)?))
}
fn scan(
    vault: &Path,
    relative: &Path,
    source: &str,
    confidence: &str,
    out: &mut BTreeMap<String, Value>,
) -> Result<()> {
    let path = safe_path(vault, relative, true)?;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                continue;
            }
            scan(
                vault,
                &relative.join(entry.file_name()),
                source,
                confidence,
                out,
            )?;
        }
        return Ok(());
    }
    if path
        .extension()
        .is_none_or(|e| !e.eq_ignore_ascii_case("md"))
    {
        return Ok(());
    }
    ensure!(
        std::fs::metadata(&path)?.len() <= 4 * 1024 * 1024,
        "template exceeds 4MiB"
    );
    let content = std::fs::read_to_string(&path)?;
    let mut headings = Vec::new();
    for line in content.lines() {
        if line.starts_with('#') {
            headings.push(line.trim_start_matches('#').trim().to_owned());
        }
    }
    let dynamic = regex::Regex::new(r"(?s)<%.*?%>")?
        .find_iter(&content)
        .map(|m| m.as_str().chars().take(120).collect::<String>())
        .collect::<Vec<_>>();
    let placeholders = regex::Regex::new(r"\{\{([^}]+)\}\}")?
        .captures_iter(&content)
        .map(|c| c[1].trim().to_owned())
        .collect::<Vec<_>>();
    let name = relative.to_string_lossy().replace('\\', "/");
    out.insert(name.clone(),json!({"path":name,"name":path.file_stem().context("template has no stem")?.to_string_lossy(),"source":source,"confidence":confidence,"headings":headings,"placeholders":placeholders,"unsupported_templater":dynamic,"dynamic_execution":false,"content_hash":crate::cache::hash(content.as_bytes())}));
    Ok(())
}
pub fn discover_templates(vault: &Path) -> Result<Value> {
    let mut candidates = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut configured = Vec::new();
    let mut mappings = Vec::new();
    match read_json(vault, ".obsidian/templates.json") {
        Ok(Some(v)) => {
            if let Some(folder) = v["folder"].as_str().filter(|s| !s.trim().is_empty()) {
                configured.push((
                    folder.trim().trim_matches('/').to_owned(),
                    "core_templates_config",
                ));
            }
        }
        Ok(None) => (),
        Err(e) => warnings.push(format!("Invalid core templates config: {e:#}")),
    }
    match read_json(vault, ".obsidian/plugins/templater-obsidian/data.json") {
        Ok(Some(v)) => {
            if let Some(folder) = v["templates_folder"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
            {
                configured.push((
                    folder.trim().trim_matches('/').to_owned(),
                    "templater_config",
                ));
            }
            if let Some(entries) = v["folder_templates"].as_array() {
                for item in entries {
                    if let (Some(folder), Some(template)) =
                        (item["folder"].as_str(), item["template"].as_str())
                    {
                        if safe_path(vault, Path::new(folder), false).is_ok() {
                            mappings.push(json!({"folder":folder,"template":template}));
                            configured.push((template.to_owned(), "templater_folder_mapping"));
                        } else {
                            warnings.push(format!("Ignored unsafe folder mapping {folder}"));
                        }
                    }
                }
            }
        }
        Ok(None) => (),
        Err(e) => warnings.push(format!("Invalid Templater config: {e:#}")),
    }
    for (folder, source) in &configured {
        if let Err(e) = scan(vault, Path::new(folder), source, "high", &mut candidates) {
            warnings.push(format!("Ignored {folder}: {e:#}"));
        }
    }
    for folder in [
        "Templates",
        "templates",
        "_templates",
        "meta/templates",
        "00 - Templates",
        "00-templates",
    ] {
        let path = PathBuf::from(folder);
        if safe_path(vault, &path, false).is_ok_and(|p| p.exists()) {
            let _ = scan(vault, &path, "common_folder", "medium", &mut candidates);
        }
    }
    for note in crate::vault::index(vault, None)? {
        let rel = serde_json::to_value(&note)?;
        let path = rel["path"].as_str().unwrap_or("");
        if path.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(path);
        if !candidates.contains_key(path)
            && candidate
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().to_lowercase().contains("template"))
        {
            let _ = scan(
                vault,
                &candidate,
                "filename_heuristic",
                "low",
                &mut candidates,
            );
        }
    }
    let mut templates: Vec<_> = candidates.into_values().collect();
    templates.sort_by_key(|v| v["path"].as_str().unwrap_or("").to_lowercase());
    Ok(
        json!({"templates":templates,"folder_mappings":mappings,"warnings":warnings,"dynamic_execution":false}),
    )
}
pub fn select_template(discovered: &Value, note_path: &Path) -> Result<Option<Value>> {
    crate::validation::relative_path(note_path)?;
    let name = note_path.to_string_lossy().replace('\\', "/");
    let mut matches = Vec::new();
    for m in discovered["folder_mappings"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let (Some(folder), Some(template)) = (m["folder"].as_str(), m["template"].as_str())
            && Path::new(&name).starts_with(folder)
        {
            matches.push((folder.len(), template));
        }
    }
    matches.sort_by_key(|(n, _)| *n);
    if let Some((_, target)) = matches.last() {
        return Ok(discovered["templates"]
            .as_array()
            .and_then(|t| t.iter().find(|v| v["path"] == *target))
            .cloned());
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_mappings_without_executing_dynamic_content() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".obsidian/plugins/templater-obsidian")).unwrap();
        std::fs::create_dir_all(d.path().join("Templates")).unwrap();
        std::fs::write(d.path().join("Templates/Deep.md"),"---\ntopic: Template\ntags: []\nmaster_topic: General\ndomain: General\nnote_type: deep-notes\n---\n# {{title}}\n<%* throw new Error('must never run') %>").unwrap();
        std::fs::write(
            d.path().join(".obsidian/templates.json"),
            r#"{"folder":"Templates"}"#,
        )
        .unwrap();
        std::fs::write(
            d.path()
                .join(".obsidian/plugins/templater-obsidian/data.json"),
            r#"{"folder_templates":[{"folder":"Notes","template":"Templates/Deep.md"}]}"#,
        )
        .unwrap();
        let result = discover_templates(d.path()).unwrap();
        assert_eq!(result["templates"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["templates"][0]["unsupported_templater"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            select_template(&result, Path::new("Notes/One.md"))
                .unwrap()
                .unwrap()["path"],
            "Templates/Deep.md"
        );
    }
}
