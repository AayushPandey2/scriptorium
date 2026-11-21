//! Prospective validation and journaled, hash-guarded updates of existing collections.
use crate::{
    cache::hash,
    planning::KnowledgePlan,
    validation::{NoteDraft, VisualReceipt, audit_collection, safe_path},
    vault::write_checked,
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn update_collection(
    vault: &Path,
    collection: &Path,
    plan: &KnowledgePlan,
    receipts: &[VisualReceipt],
    drafts: &[NoteDraft],
    expected: &BTreeMap<PathBuf, String>,
) -> Result<serde_json::Value> {
    ensure!(!drafts.is_empty(), "update needs authored drafts");
    ensure!(
        safe_path(vault, collection, true)?.is_dir(),
        "collection must exist"
    );
    let mut originals = BTreeMap::new();
    let mut replacements = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for d in drafts {
        ensure!(
            d.path.starts_with(collection) && d.path != collection,
            "draft outside collection"
        );
        ensure!(seen.insert(d.path.clone()), "duplicate update path");
        let path = safe_path(vault, &d.path, false)?;
        let bytes = if path.exists() {
            Some(fs::read(path)?)
        } else {
            None
        };
        ensure!(
            expected.get(&d.path).cloned() == bytes.as_ref().map(|b| hash(b)),
            "missing or stale expected note hash: {}",
            d.path.display()
        );
        crate::validation::validate_properties(&d.content)?;
        if bytes.as_deref() != Some(d.content.as_bytes()) {
            originals.insert(d.path.clone(), bytes);
            replacements.insert(d.path.clone(), d.content.clone());
        }
    }
    let prospective = tempfile::tempdir()?;
    let mut paths: BTreeSet<PathBuf> = plan.notes.iter().map(PathBuf::from).collect();
    paths.extend(
        plan.visuals
            .iter()
            .map(|v| PathBuf::from(&v.need.placement)),
    );
    paths.extend(drafts.iter().map(|d| d.path.clone()));
    paths.extend(receipts.iter().map(|r| r.local_path.clone()));
    for relative in paths {
        let dest = safe_path(prospective.path(), &relative, false)?;
        fs::create_dir_all(dest.parent().context("missing parent")?)?;
        if let Some(content) = replacements.get(&relative) {
            fs::write(dest, content)?;
        } else {
            fs::copy(safe_path(vault, &relative, true)?, dest)?;
        }
    }
    let audit = audit_collection(prospective.path(), plan, receipts)?;
    ensure!(
        audit.artifact_complete,
        "prospective collection validation failed: {}",
        audit.errors.join("; ")
    );
    if replacements.is_empty() {
        return Ok(json!({"complete":true,"changed":false,"audit":audit}));
    }
    let identity = hash(&serde_json::to_vec(&(collection, &replacements, expected))?);
    let relative = PathBuf::from(format!(".scriptorium/updates/{identity}"));
    let journal = safe_path(vault, &relative, false)?;
    ensure!(
        !journal.exists(),
        "update journal exists; inspect previous attempt before retrying"
    );
    fs::create_dir_all(&journal)?;
    safe_path(vault, &relative, true)?;
    for (i, (path, bytes)) in originals.iter().enumerate() {
        if let Some(bytes) = bytes {
            fs::write(journal.join(format!("backup-{i}")), bytes)?;
            ensure!(
                hash(&fs::read(safe_path(vault, path, true)?)?) == hash(bytes),
                "note changed before update journal"
            );
        } else {
            ensure!(
                !safe_path(vault, path, false)?.exists(),
                "new note appeared before journal"
            );
        }
    }
    let entries:Vec<_>=originals.iter().enumerate().map(|(i,(path,bytes))|json!({"path":path,"backup":bytes.as_ref().map(|_|format!("backup-{i}")),"old_hash":bytes.as_ref().map(|b|hash(b)),"new_hash":hash(replacements[path].as_bytes())})).collect();
    let mut prepared = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(journal.join("prepared.json"))?;
    prepared.write_all(&serde_json::to_vec_pretty(
        &json!({"state":"prepared","entries":entries}),
    )?)?;
    prepared.sync_all()?;
    let mut written = Vec::new();
    let result: Result<()> = (|| {
        for (path, content) in &replacements {
            write_checked(vault, path, content, expected.get(path).map(String::as_str))?;
            written.push(path.clone());
        }
        let verified = audit_collection(vault, plan, receipts)?;
        ensure!(verified.artifact_complete, "post-update validation failed");
        Ok(())
    })();
    if let Err(error) = result {
        let mut conflicts = Vec::new();
        for path in written.into_iter().rev() {
            let rollback: Result<()> = if let Some(old) = &originals[&path] {
                write_checked(
                    vault,
                    &path,
                    std::str::from_utf8(old)?,
                    Some(&hash(replacements[&path].as_bytes())),
                )
                .map(|_| ())
            } else {
                (|| {
                    let target = safe_path(vault, &path, true)?;
                    ensure!(
                        hash(&fs::read(&target)?) == hash(replacements[&path].as_bytes()),
                        "new note changed during rollback"
                    );
                    fs::remove_file(target)?;
                    Ok(())
                })()
            };
            if let Err(e) = rollback {
                conflicts.push(format!("{}: {e}", path.display()));
            }
        }
        fs::write(
            journal.join("failed.json"),
            serde_json::to_vec_pretty(
                &json!({"state":"failed","error":format!("{error:#}"),"rollback_conflicts":conflicts}),
            )?,
        )?;
        return Err(error.context(format!(
            "journal {}; rollback conflicts: {}",
            journal.display(),
            conflicts.join("; ")
        )));
    }
    let note_hashes: BTreeMap<_, _> = drafts
        .iter()
        .map(|d| (d.path.clone(), hash(d.content.as_bytes())))
        .collect();
    let report = json!({"complete":true,"changed":true,"publication_state":"published","plan_sha256":hash(&serde_json::to_vec(plan)?),"notes":note_hashes,"visuals":receipts,"updated":replacements.keys().collect::<Vec<_>>(),"audit":audit,"journal":relative});
    let mut committed = tempfile::NamedTempFile::new_in(&journal)?;
    committed.write_all(&serde_json::to_vec_pretty(&report)?)?;
    committed.as_file().sync_all()?;
    committed.persist_noclobber(journal.join("committed.json"))?;
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_chapters_can_join_prepared_attachment_directory() {
        let vault = tempfile::tempdir().unwrap();
        fs::create_dir_all(vault.path().join("Topic/assets")).unwrap();
        let note = "---\ntopic: T\ntags: [topic/t]\nmaster_topic: T\ndomain: General\nnote_type: deep-notes\n---\nAuthored chapter\n";
        let plan:KnowledgePlan=serde_json::from_value(json!({"topic":"T","notes":["Topic/01.md"],"concepts":[{"id":"t","title":"T","objectives":["Explain T"]}]})).unwrap();
        let drafts = vec![NoteDraft {
            path: "Topic/01.md".into(),
            content: note.into(),
        }];
        let report = update_collection(
            vault.path(),
            Path::new("Topic"),
            &plan,
            &[],
            &drafts,
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(report["publication_state"], "published");
        assert!(vault.path().join("Topic/01.md").is_file());
    }
    #[test]
    fn stale_replacements_do_not_change_notes() {
        let vault = tempfile::tempdir().unwrap();
        fs::create_dir(vault.path().join("Topic")).unwrap();
        let note = "---\ntopic: T\ntags: [topic/t]\nmaster_topic: T\ndomain: General\nnote_type: deep-notes\n---\nOld\n";
        fs::write(vault.path().join("Topic/01.md"), note).unwrap();
        let plan:KnowledgePlan=serde_json::from_value(json!({"topic":"T","notes":["Topic/01.md"],"concepts":[{"id":"t","title":"T","objectives":["Explain T"]}]})).unwrap();
        let drafts = vec![NoteDraft {
            path: "Topic/01.md".into(),
            content: note.replace("Old", "New"),
        }];
        let stale = BTreeMap::from([(PathBuf::from("Topic/01.md"), "wrong".into())]);
        assert!(
            update_collection(
                vault.path(),
                Path::new("Topic"),
                &plan,
                &[],
                &drafts,
                &stale
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(vault.path().join("Topic/01.md")).unwrap(),
            note
        );
        let good = BTreeMap::from([(PathBuf::from("Topic/01.md"), hash(note.as_bytes()))]);
        let result =
            update_collection(vault.path(), Path::new("Topic"), &plan, &[], &drafts, &good)
                .unwrap();
        assert_eq!(result["complete"], true);
        assert!(
            fs::read_to_string(vault.path().join("Topic/01.md"))
                .unwrap()
                .contains("New")
        );
    }
}
