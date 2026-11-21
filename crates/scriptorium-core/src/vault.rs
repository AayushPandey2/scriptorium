//! Vault discovery, graph analysis and conflict-aware note edits.
use crate::{cache::hash, validation::safe_path};
pub fn set_notes_folder(folder: &Path, configuration: &Path) -> Result<Value> {
    fs::create_dir_all(folder)?;
    let notes = folder.canonicalize()?;
    let root = notes
        .ancestors()
        .find(|p| p.join(".obsidian").is_dir())
        .unwrap_or(&notes);
    let relative = notes
        .strip_prefix(root)?
        .to_string_lossy()
        .replace('\\', "/");
    let value = json!({"vault_path":root,"notes_folder":if relative.is_empty(){"."}else{&relative},"notes_path":notes});
    let parent = configuration
        .parent()
        .context("configuration needs a parent")?;
    fs::create_dir_all(parent)?;
    if configuration.exists() {
        ensure!(
            !fs::symlink_metadata(configuration)?
                .file_type()
                .is_symlink(),
            "config symlink rejected"
        );
    }
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(&serde_json::to_vec_pretty(&value)?)?;
    staged.as_file().sync_all()?;
    staged.persist(configuration)?;
    Ok(value)
}
pub fn resolve_vault(explicit: Option<&Path>, configuration: &Path, start: &Path) -> Result<Value> {
    let config: Value = if configuration.exists() {
        serde_json::from_slice(&fs::read(configuration)?)?
    } else {
        json!({})
    };
    let env = std::env::var_os("SCRIPTORIUM_VAULT_PATH").map(std::path::PathBuf::from);
    let configured = config["vault_path"].as_str().map(Path::new);
    let detected = start.ancestors().find(|p| p.join(".obsidian").is_dir());
    let root = explicit
        .or(env.as_deref())
        .or(configured)
        .or(detected)
        .context("supply a vault, SCRIPTORIUM_VAULT_PATH, configuration, or .obsidian ancestor")?
        .canonicalize()?;
    ensure!(root.is_dir(), "vault must be a directory");
    let notes = if configured.and_then(|p| p.canonicalize().ok()).as_ref() == Some(&root) {
        let folder = config["notes_folder"].as_str().unwrap_or(".");
        if folder == "." {
            root.clone()
        } else {
            safe_path(&root, Path::new(folder), false)?
        }
    } else {
        root.clone()
    };
    Ok(json!({"vault_path":root,"notes_path":notes,"configuration":configuration}))
}
use anyhow::{Context, Result, ensure};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedNote {
    pub file: String,
    pub title: String,
    pub properties: Value,
    pub links: Vec<String>,
    pub hash: String,
}
pub fn frontmatter(text: &str) -> Result<(Value, &str)> {
    let mut lines = text.split_inclusive('\n');
    ensure!(
        lines
            .next()
            .is_some_and(|s| s.trim_start_matches('\u{feff}').trim() == "---"),
        "missing YAML Properties"
    );
    let mut offset = text.split_inclusive('\n').next().unwrap_or("").len();
    let start = offset;
    for line in lines {
        if line.trim() == "---" {
            let properties: Value = serde_yaml_ng::from_str(&text[start..offset])?;
            ensure!(properties.is_object(), "Properties must be a mapping");
            return Ok((properties, &text[offset + line.len()..]));
        }
        offset += line.len();
    }
    anyhow::bail!("unclosed Properties");
}
pub fn protected_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut block = None;
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => block = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = block.take() {
                    ranges.push(start..range.end)
                }
            }
            Event::Code(_) | Event::Html(_) | Event::InlineHtml(_) => ranges.push(range),
            _ => (),
        }
    }
    ranges
}
pub fn wikilinks(text: &str) -> Vec<String> {
    let protected = protected_ranges(text);
    let re = regex::Regex::new(r"\[\[([^\]\r\n]+)\]\]").unwrap();
    re.captures_iter(text)
        .filter(|c| {
            !protected
                .iter()
                .any(|r| r.contains(&c.get(0).unwrap().start()))
        })
        .map(|c| {
            c[1].split(['|', '#'])
                .next()
                .unwrap_or("")
                .trim()
                .to_owned()
        })
        .filter(|s| !s.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub fn index(vault: &Path, topic: Option<&str>) -> Result<Vec<IndexedNote>> {
    let root = vault.canonicalize()?;
    let mut pending = vec![root.clone()];
    let mut notes = Vec::new();
    while let Some(dir) = pending.pop() {
        let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                ensure!(
                    path.canonicalize()?.starts_with(&root),
                    "vault symlink escapes root"
                );
                continue;
            }
            if kind.is_dir() {
                if ![
                    ".git",
                    ".trash",
                    ".obsidian",
                    ".scriptorium",
                    "node_modules",
                    "__pycache__",
                    "scratchpad",
                    "attachments",
                    "target",
                ]
                .contains(&entry.file_name().to_string_lossy().as_ref())
                {
                    pending.push(path)
                }
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            let properties = frontmatter(&text).map(|(p, _)| p).unwrap_or(json!({}));
            let title = properties["title"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| path.file_stem().unwrap().to_string_lossy().into_owned());
            if let Some(t) = topic
                && !format!("{title} {}", properties)
                    .to_lowercase()
                    .contains(&t.to_lowercase())
            {
                continue;
            }
            notes.push(IndexedNote {
                file: path
                    .strip_prefix(&root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                title,
                properties,
                links: wikilinks(&text),
                hash: hash(text.as_bytes()),
            });
        }
    }
    notes.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(notes)
}
fn target(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .split(['|', '#'])
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase()
}
pub fn graph(notes: &[IndexedNote]) -> Value {
    let mut lookup: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for n in notes {
        let mut names = vec![
            n.file.clone(),
            n.file.trim_end_matches(".md").into(),
            Path::new(&n.file)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            n.title.clone(),
        ];
        if let Some(s) = n.properties["id"].as_str() {
            names.push(s.into())
        }
        if let Some(a) = n.properties["aliases"].as_array() {
            names.extend(a.iter().filter_map(Value::as_str).map(str::to_owned))
        }
        for name in names {
            lookup
                .entry(name.to_lowercase())
                .or_default()
                .insert(n.file.clone());
        }
    }
    let mut edges = BTreeSet::new();
    let mut broken = Vec::new();
    let mut ambiguous = Vec::new();
    let mut deps: BTreeMap<String, BTreeSet<String>> = notes
        .iter()
        .map(|n| (n.file.clone(), BTreeSet::new()))
        .collect();
    for n in notes {
        let mut links = n.links.clone();
        for field in ["depends_on", "prerequisites", "nexttopics", "next_topic"] {
            let v = &n.properties[field];
            let values: Vec<_> = if let Some(a) = v.as_array() {
                a.iter().filter_map(Value::as_str).collect()
            } else {
                v.as_str().into_iter().collect()
            };
            for s in values {
                links.push(s.into());
                if let Some(found) = lookup.get(&target(s))
                    && found.len() == 1
                {
                    let dest = found.first().unwrap();
                    if field == "depends_on" || field == "prerequisites" {
                        deps.get_mut(&n.file).unwrap().insert(dest.clone());
                    } else {
                        deps.get_mut(dest).unwrap().insert(n.file.clone());
                    }
                }
            }
        }
        for link in links {
            match lookup.get(&target(&link)) {
                Some(found) if found.len() == 1 => {
                    edges.insert((n.file.clone(), found.first().unwrap().clone()));
                }
                Some(found) => {
                    ambiguous.push(json!({"from":n.file,"target":link,"candidates":found}))
                }
                None => broken.push(json!({"from":n.file,"target":link})),
            }
        }
    }
    let mut remaining = deps.clone();
    let mut order = Vec::new();
    loop {
        let ready: Vec<_> = remaining
            .iter()
            .filter(|(_, v)| v.is_empty())
            .map(|(k, _)| k.clone())
            .collect();
        if ready.is_empty() {
            break;
        }
        for id in ready {
            remaining.remove(&id);
            for values in remaining.values_mut() {
                values.remove(&id);
            }
            order.push(id);
        }
    }
    let incoming: BTreeSet<_> = edges.iter().map(|(_, b)| b.clone()).collect();
    let outgoing: BTreeSet<_> = edges.iter().map(|(a, _)| a.clone()).collect();
    json!({"nodes":notes,"edges":edges,"dependencies":deps,"topo_order":order,"cycle_affected":remaining.keys().collect::<Vec<_>>(),"acyclic":remaining.is_empty(),"broken_links":broken,"ambiguous_links":ambiguous,"orphaned":notes.iter().filter(|n|!incoming.contains(&n.file)).map(|n|&n.file).collect::<Vec<_>>(),"isolated":notes.iter().filter(|n|!incoming.contains(&n.file)&&!outgoing.contains(&n.file)).map(|n|&n.file).collect::<Vec<_>>()})
}
pub fn patch_properties(
    text: &str,
    changes: &BTreeMap<String, Value>,
    add_tags: &[String],
) -> Result<String> {
    let (mut p, body) = frontmatter(text)?;
    for (k, v) in changes {
        p[k] = v.clone();
    }
    let mut tags: BTreeSet<String> = p["tags"]
        .as_array()
        .context("tags must be an array")?
        .iter()
        .map(|v| v.as_str().context("tag must be string").map(str::to_owned))
        .collect::<Result<_>>()?;
    tags.extend(
        add_tags
            .iter()
            .map(|s| s.trim_start_matches('#').to_owned()),
    );
    p["tags"] = json!(tags);
    let (original, _) = frontmatter(text)?;
    if p == original {
        return Ok(text.into());
    }
    Ok(format!("---\n{}---\n{body}", serde_yaml_ng::to_string(&p)?))
}
pub fn rename_link(text: &str, old: &str, new: &str) -> Result<String> {
    ensure!(
        !new.is_empty() && !new.contains(['[', ']', '\r', '\n']),
        "invalid link replacement"
    );
    let protected = protected_ranges(text);
    let re = regex::Regex::new(r"\[\[([^\]\r\n]+)\]\]")?;
    let mut out = text.to_owned();
    let mut edits = Vec::new();
    for c in re.captures_iter(text) {
        let whole = c.get(0).unwrap();
        if protected.iter().any(|r| r.contains(&whole.start())) {
            continue;
        }
        let inner = &c[1];
        if target(inner) == target(old) {
            let suffix = inner.find(['|', '#']).map(|i| &inner[i..]).unwrap_or("");
            edits.push((whole.range(), format!("[[{new}{suffix}]]")));
        }
    }
    for (range, replacement) in edits.into_iter().rev() {
        out.replace_range(range, &replacement)
    }
    Ok(out)
}
pub fn write_checked(
    vault: &Path,
    relative: &Path,
    content: &str,
    expected_hash: Option<&str>,
) -> Result<bool> {
    let path = safe_path(vault, relative, false)?;
    let previous = if path.exists() {
        Some(fs::read(&path)?)
    } else {
        None
    };
    if let Some(bytes) = &previous {
        ensure!(
            expected_hash == Some(hash(bytes).as_str()),
            "existing note needs matching expected hash"
        );
        if bytes == content.as_bytes() {
            return Ok(false);
        }
    } else {
        ensure!(expected_hash.is_none(), "expected existing note is missing");
    }
    let parent = path.parent().context("note parent missing")?;
    fs::create_dir_all(parent)?;
    safe_path(vault, relative, false)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(content.as_bytes())?;
    staged.as_file().sync_all()?;
    if let Some(bytes) = previous {
        ensure!(fs::read(&path)? == bytes, "note changed while staging");
        staged.persist(&path)?;
    } else {
        staged.persist_noclobber(&path)?;
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_edit_preserves_body_and_custom_fields() {
        let text = "---\ntopic: Test\ntags: [one]\ncustom: keep\n---\nBody\r\nexact\n";
        let p = patch_properties(text, &BTreeMap::new(), &["two".into()]).unwrap();
        assert_eq!(frontmatter(&p).unwrap().1, "Body\r\nexact\n");
        assert_eq!(frontmatter(&p).unwrap().0["custom"], "keep");
    }
    #[test]
    fn rename_preserves_aliases_and_code() {
        let s = "[[Old#Heading|Alias]] `[[Old]]`\n```md\n[[Old]]\n```\n<!-- [[Old]] -->";
        let out = rename_link(s, "Old", "New").unwrap();
        assert!(out.contains("[[New#Heading|Alias]]"));
        assert!(out.contains("`[[Old]]`"));
        assert!(out.contains("<!-- [[Old]] -->"));
    }
    #[test]
    fn checked_write_rejects_stale_hash() {
        let d = tempfile::tempdir().unwrap();
        write_checked(d.path(), Path::new("note.md"), "one", None).unwrap();
        assert!(write_checked(d.path(), Path::new("note.md"), "two", Some("wrong")).is_err());
        assert!(write_checked(d.path(), Path::new("note.md"), "two", Some(&hash(b"one"))).unwrap());
    }
}
