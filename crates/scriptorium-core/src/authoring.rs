//! Model-assisted authored-note publication for all seven master products.
use crate::{
    cache::hash,
    metadata::{PropertiesOptions, properties, render_note},
    planning::{KnowledgePlan, build_plan},
    sessions::Session,
    validation::{NoteDraft, VisualReceipt, audit_collection, safe_path},
    vault::write_checked,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub fn generate_session(
    vault: &Path,
    session: &Session,
    drafts: &Value,
    evidence: &Value,
    reviews: &Value,
    expected: &BTreeMap<PathBuf, String>,
) -> Result<Value> {
    ensure!(
        session.stage == "COLLECTION_PLAN_READY",
        "configuration and outline approval required before generation"
    );
    let prefs = session
        .resolved_preferences
        .as_ref()
        .context("missing resolved preferences")?;
    let collection = session
        .collection_plan
        .as_ref()
        .context("missing collection plan")?;
    let input = drafts
        .get("drafts")
        .unwrap_or(drafts)
        .as_object()
        .context("drafts must map concept IDs to authored Markdown bodies")?;
    let mut authored = BTreeMap::new();
    for n in collection
        .nodes
        .iter()
        .filter(|n| n.note_role != "root_index" && n.note_role != "group_index")
    {
        let value = input
            .get(&n.concept_id)
            .or_else(|| input.get(&n.proposed_path))
            .context(format!("missing authored body for {}", n.concept_id))?;
        let body = value
            .as_str()
            .or_else(|| value["body"].as_str())
            .context("draft must be string or object with body")?;
        ensure!(!body.trim().is_empty(), "empty authored body");
        let errors =
            crate::quality::validate_master_output(body, &prefs.master, prefs.variant.as_deref());
        ensure!(errors.is_empty(), "{}: {}", n.concept_id, errors.join("; "));
        let structure = crate::composition::validate_chapter(body, &[]);
        ensure!(
            structure.is_empty(),
            "{}: {}",
            n.concept_id,
            structure.join("; ")
        );
        authored.insert(n.concept_id.clone(), body.to_owned());
    }
    ensure!(!authored.is_empty(), "no authored notes supplied");
    let review_records: Vec<&Value> = if let Some(a) = reviews.as_array() {
        a.iter().collect()
    } else if reviews.get("result").is_some() {
        vec![reviews]
    } else {
        reviews
            .as_object()
            .context("reviews mapping required")?
            .values()
            .collect()
    };
    ensure!(
        !review_records.is_empty(),
        "artifact-bound editorial reviews required"
    );
    let mut reviewed = BTreeSet::new();
    for review in review_records {
        let errors = crate::composition::receipt_errors(review, &authored);
        ensure!(errors.is_empty(), "review rejected: {}", errors.join("; "));
        for item in review["evidence"].as_array().unwrap() {
            if let Some(id) = item["chapter"].as_str() {
                reviewed.insert(id.to_owned());
            }
        }
    }
    ensure!(
        authored.keys().all(|id| reviewed.contains(id)),
        "every authored note needs exact-content review evidence"
    );
    let registry = if !evidence["registry"].is_null() {
        Some(crate::evidence::normalize_evidence_registry(
            &json!({"registry":evidence["registry"],"mode":if prefs.source_policy=="strict"{"R-S0"}else{"R-I"}}),
        )?)
    } else {
        None
    };
    if prefs.source_policy == "strict" || prefs.source_policy == "source_first" {
        let registry = registry
            .as_ref()
            .context("source-bound notes require a structured evidence registry")?;
        ensure!(
            registry["sources"]
                .as_array()
                .is_some_and(|s| !s.is_empty()),
            "source-bound notes require supplied source records"
        );
    }
    let mut documents = Vec::new();
    for n in &collection.nodes {
        let role = n.note_role.as_str();
        let body = if role == "root_index" || role == "group_index" {
            let mut out = format!("# {}\n\n## Learning order\n", n.title);
            for child in &n.child_ids {
                let node = collection
                    .nodes
                    .iter()
                    .find(|v| &v.concept_id == child)
                    .context("unknown outline child")?;
                out.push_str(&format!(
                    "- [[{}|{}]]\n",
                    node.proposed_path.trim_end_matches(".md"),
                    node.title
                ));
            }
            out.push_str("\n## Scope\nThe linked notes follow the approved authored outline.\n");
            out
        } else {
            authored[&n.concept_id].clone()
        };
        let props = properties(
            &n.title,
            if role == "root_index" || role == "group_index" {
                "index"
            } else {
                crate::orchestration::note_type(&prefs.master)
            },
            &PropertiesOptions {
                parent_topic: (role != "single" && role != "root_index")
                    .then(|| collection.title.clone()),
                collection_id: Some(collection.collection_id.clone()),
                optional: BTreeMap::from([
                    ("source_mode".into(), json!(prefs.source_policy)),
                    ("concept_ids".into(), json!([n.concept_id])),
                ]),
                ..Default::default()
            },
        )?;
        documents.push(NoteDraft {
            path: PathBuf::from(&n.proposed_path),
            content: render_note(&props, &body)?,
        });
    }
    let receipts: Vec<VisualReceipt> = if evidence["visual_receipts"].is_null() {
        Vec::new()
    } else {
        serde_json::from_value(evidence["visual_receipts"].clone())?
    };
    let plan: KnowledgePlan = if evidence["visual_plan"].is_null() {
        serde_json::from_value(
            json!({"topic":collection.title,"notes":documents.iter().map(|n|n.path.to_string_lossy().replace('\\',"/")).collect::<Vec<_>>(),"source_policy":prefs.source_policy,"concepts":collection.nodes.iter().map(|n|json!({"id":n.concept_id,"title":n.title,"objectives":[prefs.learning_objective],"complexity":1})).collect::<Vec<_>>()}),
        )?
    } else {
        serde_json::from_value(evidence["visual_plan"].clone())?
    };
    let plan = build_plan(plan)?;
    let prospective = tempfile::tempdir()?;
    let mut originals = BTreeMap::new();
    for draft in &documents {
        let path = safe_path(vault, &draft.path, false)?;
        let old = if path.exists() {
            let b = std::fs::read(&path)?;
            ensure!(
                expected.get(&draft.path) == Some(&hash(&b)),
                "missing or stale expected note hash: {}",
                draft.path.display()
            );
            Some(b)
        } else {
            None
        };
        originals.insert(draft.path.clone(), old);
        let target = safe_path(prospective.path(), &draft.path, false)?;
        std::fs::create_dir_all(target.parent().context("missing parent")?)?;
        std::fs::write(target, &draft.content)?;
    }
    for r in &receipts {
        let source = safe_path(vault, &r.local_path, true)?;
        let target = safe_path(prospective.path(), &r.local_path, false)?;
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(source, target)?;
    }
    let audit = audit_collection(prospective.path(), &plan, &receipts)?;
    ensure!(
        audit.artifact_complete,
        "artifact fulfillment blocked: {}",
        audit.errors.join("; ")
    );
    let manifest_relative = PathBuf::from(format!(
        ".scriptorium/collections/{}.json",
        session.request_id
    ));
    let manifest_path = safe_path(vault, &manifest_relative, false)?;
    let manifest_expected = if manifest_path.exists() {
        Some(hash(&std::fs::read(&manifest_path)?))
    } else {
        None
    };
    let entries: Vec<_> = documents
        .iter()
        .map(|d| json!({"path":d.path,"hash":hash(d.content.as_bytes())}))
        .collect();
    let prepared = json!({"session_id":session.request_id,"collection_id":collection.collection_id,"state":"prepared","notes":entries,"source_policy":prefs.source_policy,"reviewed_concepts":reviewed,"visual_receipts":receipts,"audit":audit});
    let journal_relative = PathBuf::from(format!(
        ".scriptorium/authoring/{}-{}.prepared.json",
        session.request_id,
        &hash(&serde_json::to_vec(&prepared)?)[..16]
    ));
    write_checked(
        vault,
        &journal_relative,
        &serde_json::to_string_pretty(&prepared)?,
        None,
    )?;
    let mut written: Vec<&NoteDraft> = Vec::new();
    for document in &documents {
        if let Err(error) = write_checked(
            vault,
            &document.path,
            &document.content,
            expected.get(&document.path).map(String::as_str),
        ) {
            for old in written.iter().rev() {
                let current = safe_path(vault, &old.path, true)?;
                if hash(&std::fs::read(&current)?) == hash(old.content.as_bytes()) {
                    if let Some(bytes) = &originals[&old.path] {
                        write_checked(
                            vault,
                            &old.path,
                            std::str::from_utf8(bytes)?,
                            Some(&hash(old.content.as_bytes())),
                        )?;
                    } else {
                        std::fs::remove_file(current)?;
                    }
                }
            }
            return Err(error.context("authoring publication failed; prepared journal retained"));
        }
        written.push(document);
    }
    let mut committed = prepared;
    committed["state"] = json!("published");
    committed["educational_review"] = json!(
        "artifact-bound editorial attestations; scientific truth and learner mastery not inferred"
    );
    write_checked(
        vault,
        &manifest_relative,
        &serde_json::to_string_pretty(&committed)?,
        manifest_expected.as_deref(),
    )?;
    Ok(
        json!({"complete":true,"publication_state":"published","manifest_path":manifest_relative,"notes":entries,"audit":audit,"session_id":session.request_id}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_single_note_rejects_stale_reviews_and_preserves_user_edits() {
        let session = crate::sessions::respond(
            crate::sessions::start("formula sheet for mechanics", false).unwrap(),
            "use your recommendations",
        )
        .unwrap()
        .state;
        let cid = session.collection_plan.as_ref().unwrap().nodes[0]
            .concept_id
            .clone();
        let body = "# Mechanics\n\n## Equations\n- $F=ma$: force equals mass times acceleration.\n";
        let drafts = json!({cid.clone():body});
        let review = json!({"verifier":"independent reviewer","method":"inspect formula and symbols","limitations":"ideal classical regime","result":"pass","evidence":[{"chapter":cid,"hash":hash(body.as_bytes()),"excerpt":"force equals mass"}]});
        let dir = tempfile::tempdir().unwrap();
        assert!(
            generate_session(
                dir.path(),
                &session,
                &drafts,
                &json!({}),
                &review,
                &BTreeMap::new()
            )
            .is_ok()
        );
        assert!(
            generate_session(
                dir.path(),
                &session,
                &drafts,
                &json!({}),
                &review,
                &BTreeMap::new()
            )
            .is_err()
        );
        let mut stale = review;
        stale["evidence"][0]["hash"] = json!("stale");
        assert!(
            generate_session(
                dir.path(),
                &session,
                &drafts,
                &json!({}),
                &stale,
                &BTreeMap::new()
            )
            .is_err()
        );
    }
}
