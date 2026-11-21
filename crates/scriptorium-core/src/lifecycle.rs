//! Collection depth, reuse, prerequisites, checkpoints and explicit reader repairs.
use crate::{
    cache::hash,
    knowledge::{compute_change_impact, detect_semantic_changes, normalize, strings},
    validation::{safe_path, validate_properties},
    vault,
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};
fn nodes(v: &Value) -> Result<&Vec<Value>> {
    let plan = v.get("plan").unwrap_or(v);
    plan["nodes"]
        .as_array()
        .or(plan["concepts"].as_array())
        .context("collection nodes/concepts required")
}
fn id(n: &Value) -> Result<&str> {
    n["concept_id"]
        .as_str()
        .or(n["id"].as_str())
        .context("concept id required")
}
fn title(n: &Value) -> &str {
    n["title"].as_str().unwrap_or("")
}
fn prerequisites(n: &Value) -> Vec<String> {
    if n["prerequisite_ids"].is_array() {
        strings(&n["prerequisite_ids"])
    } else {
        strings(&n["prerequisites"])
    }
}
fn path(n: &Value) -> Result<&str> {
    n["proposed_path"]
        .as_str()
        .or(n["path"].as_str())
        .context("proposed note path required")
}
pub fn allocate_depth(input: &Value) -> Result<Value> {
    let nodes = nodes(input)?;
    let depth = input["preferences"]["depth"]
        .as_str()
        .or(input["depth"].as_str())
        .unwrap_or("standard");
    let priorities = strings(&input["user_priorities"]);
    let mut dependents = BTreeMap::<String, usize>::new();
    for n in nodes {
        for p in prerequisites(n) {
            *dependents.entry(p).or_default() += 1;
        }
    }
    let mut allocations = BTreeMap::new();
    let mut low = Vec::new();
    for n in nodes {
        if matches!(n["note_role"].as_str(), Some("root_index" | "group_index")) {
            continue;
        }
        let cid = id(n)?;
        let count = dependents.get(cid).copied().unwrap_or(0);
        let lower = title(n).to_lowercase();
        let complexity = n["complexity"].as_u64().unwrap_or(2);
        let mut score = 1i64 + count.min(2) as i64;
        if complexity >= 4 {
            score += 2;
        }
        if ["overview", "introduction", "orientation"]
            .iter()
            .any(|s| lower.contains(s))
        {
            score -= 1;
        }
        if priorities
            .iter()
            .any(|p| !p.is_empty() && lower.contains(&p.to_lowercase()))
        {
            score += 3;
        }
        let allocation = match depth {
            "concise" => {
                if score >= 3 {
                    "standard"
                } else {
                    "orientation"
                }
            }
            "exhaustive" => {
                if score >= 3 {
                    "exhaustive"
                } else if score >= 2 {
                    "deep"
                } else {
                    "detailed"
                }
            }
            "comprehensive" => {
                if score >= 3 {
                    "deep"
                } else if score >= 2 {
                    "detailed"
                } else if score >= 1 {
                    "standard"
                } else {
                    "orientation"
                }
            }
            _ => {
                if score >= 3 {
                    "detailed"
                } else if score >= 1 {
                    "standard"
                } else {
                    "orientation"
                }
            }
        };
        if score < 1 {
            low.push(cid.to_owned());
        }
        allocations.insert(cid.to_owned(),json!({"concept_id":cid,"title":title(n),"priority":if score>=3{"high"}else if score>=1{"medium"}else{"low"},"score":score,"allocation":allocation,"emphasis":{"derivation":if complexity>=4&&score>=3{"deep"}else{"standard"},"numerical_examples":if score>=3{"detailed"}else{"standard"},"implementation":if input["preferences"]["master"]=="M5"{"detailed"}else{"optional"}},"expose_scores_in_note":false}));
    }
    let budget = input["budget_notes"].as_u64().map(|n| n as usize);
    let excess = budget
        .map(|b| allocations.len().saturating_sub(b))
        .unwrap_or(0);
    let deferred: Vec<_> = low.into_iter().take(excess).collect();
    for cid in &deferred {
        allocations.get_mut(cid).unwrap()["deferred_optional_expansion"] = json!(true);
    }
    Ok(
        json!({"requested_depth":depth,"budget_notes":budget,"allocations":allocations,"deferred_optional":deferred,"warnings":if excess>0{vec!["Content exceeds single-pass budget; preserve essential concepts and checkpoint remaining work"]}else{vec![]}}),
    )
}
pub fn learning_paths(input: &Value) -> Result<Value> {
    let all = nodes(input)?;
    let mut map = BTreeMap::new();
    for n in all {
        ensure!(
            map.insert(id(n)?.to_owned(), n).is_none(),
            "duplicate concept id"
        );
    }
    let order = strings(&input.get("plan").unwrap_or(input)["learning_order"]);
    let known: BTreeSet<_> = strings(&input["known_concepts"])
        .iter()
        .map(|s| normalize(s))
        .collect();
    let mut pending: Vec<_> = map.keys().cloned().collect();
    pending.sort_by_key(|cid| {
        (
            order.iter().position(|id| id == cid).unwrap_or(usize::MAX),
            cid.clone(),
        )
    });
    let mut done = BTreeSet::new();
    let mut selected = Vec::new();
    let mut cycles = Vec::new();
    while !pending.is_empty() {
        if let Some(index) = pending.iter().position(|cid| {
            prerequisites(map[cid])
                .iter()
                .all(|p| !map.contains_key(p) || done.contains(p))
        }) {
            let cid = pending.remove(index);
            done.insert(cid.clone());
            selected.push(cid);
        } else {
            cycles.push(pending.clone());
            selected.append(&mut pending);
        }
    }
    let steps:Vec<_>=selected.iter().enumerate().map(|(i,cid)|{let n=map[cid];json!({"step":i+1,"concept_id":cid,"title":title(n),"path":path(n).ok(),"prerequisite_ids":prerequisites(n),"skippable_known_concept":known.contains(&normalize(title(n)))||known.contains(&normalize(cid)),"keep_prerequisite_link":true})}).collect();
    Ok(
        json!({"strategy":input["strategy"].as_str().unwrap_or("prerequisite_order"),"active_reading_order":steps.iter().filter(|s|s["skippable_known_concept"]!=true).map(|s|&s["concept_id"]).collect::<Vec<_>>(),"full_prerequisite_order":selected,"steps":steps,"cycles_detected":cycles,"distinct_from_folder_hierarchy":true}),
    )
}
pub fn prerequisite_gaps(input: &Value) -> Result<Value> {
    let all = nodes(input)?;
    let map: BTreeMap<_, _> = all
        .iter()
        .map(|n| Ok((id(n)?.to_owned(), n)))
        .collect::<Result<_>>()?;
    let known: BTreeSet<_> = strings(&input["known_concepts"])
        .iter()
        .chain(strings(&input["vault_titles"]).iter())
        .map(|s| normalize(s))
        .collect();
    let order = strings(&input.get("plan").unwrap_or(input)["learning_order"]);
    let mut gaps = Vec::new();
    for n in all {
        let cid = id(n)?;
        let deps = prerequisites(n);
        if deps.len() > 5 {
            gaps.push(json!({"gap_type":"G5","concept_id":cid,"recommended_action":"prune_to_essential_prerequisites"}));
        }
        for dep in deps {
            if known.contains(&normalize(&dep)) {
                continue;
            }
            if let Some(p) = map.get(&dep) {
                let before = order.iter().position(|id| id == &dep);
                let after = order.iter().position(|id| id == cid);
                if before.zip(after).is_some_and(|(a, b)| a >= b) {
                    gaps.push(json!({"gap_type":"G4","concept_id":cid,"prerequisite":dep,"recommended_action":"reorder_prerequisite_before_dependent"}));
                }
                if known.contains(&normalize(title(p))) {
                    continue;
                }
            } else {
                gaps.push(json!({"gap_type":"G1","concept_id":cid,"prerequisite":dep,"recommended_action":"inline_bridge_or_explicit_prerequisite_note","source_policy":input["source_policy"]}));
            }
        }
    }
    Ok(json!({"gaps":gaps,"has_gaps":!gaps.is_empty(),"diagnostic_quiz_required":false}))
}
pub fn reconcile_notes(input: &Value) -> Result<Value> {
    let vault_root = Path::new(input["vault"].as_str().context("vault required")?);
    let notes = vault::index(vault_root, None)?;
    let mut decisions = BTreeMap::new();
    for n in nodes(input)? {
        let cid = id(n)?;
        let candidates: Vec<_> = notes
            .iter()
            .filter(|note| {
                let registered=strings(&note.properties["concept_ids"]);
                let identity=if registered.is_empty(){normalize(&note.title)==normalize(title(n))}else{registered.iter().any(|id|id==cid)};
                identity&&n["domain"].as_str().is_none_or(|domain|note.properties["domain"].as_str().is_some_and(|observed|normalize(domain)==normalize(observed)))
            })
            .collect();
        let mut entry = if candidates.len() > 1 {
            json!({"decision":"manual_review","reason":"ambiguous identity","candidates":candidates.iter().map(|n|&n.file).collect::<Vec<_>>()})
        } else if let Some(note) = candidates.first() {
            let current = fs::read_to_string(safe_path(vault_root, Path::new(&note.file), true)?)?;
            let expected = n["expected_hash"].as_str();
            if expected.is_some_and(|hash| hash != note.hash) {
                json!({"decision":"manual_review","reason":"manually edited note","matched_path":note.file,"sha256":note.hash})
            } else if n["content"].as_str().is_none_or(|body| body == current) {
                json!({"decision":"reuse_unchanged","matched_path":note.file,"sha256":note.hash})
            } else {
                json!({"decision":"extend_existing","matched_path":note.file,"sha256":note.hash,"requires_expected_hash":true})
            }
        } else {
            json!({"decision":"create_new","proposed_path":path(n)?})
        };
        entry["concept_id"] = json!(cid);
        decisions.insert(cid, entry);
    }
    Ok(json!({"decisions":decisions,"preserves_unrelated_notes":true}))
}
fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path.parent().context("checkpoint parent missing")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&serde_json::to_vec_pretty(value)?)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
/// Persist dependency-aware checkpoints for supplied authored notes. No placeholder
/// body is generated when authoring evidence is absent.
pub fn run_checkpoint(input: &Value) -> Result<Value> {
    let vault_root = Path::new(input["vault"].as_str().context("vault required")?);
    let collection = input["collection_id"]
        .as_str()
        .context("collection_id required")?;
    ensure!(
        !collection.is_empty()
            && collection
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "unsafe collection identity"
    );
    let all = nodes(input)?;
    ensure!(!all.is_empty(), "empty collection");
    let checkpoint_relative = format!(".scriptorium/collections/{collection}.checkpoint.json");
    let checkpoint = safe_path(vault_root, Path::new(&checkpoint_relative), false)?;
    let previous: Value = if checkpoint.exists() && input["resume"] != false {
        serde_json::from_slice(&fs::read(&checkpoint)?)
            .context("corrupt checkpoint requires explicit repair")?
    } else {
        json!({})
    };
    let sequence = learning_paths(input)?;
    ensure!(
        sequence["cycles_detected"]
            .as_array()
            .is_some_and(|a| a.is_empty()),
        "cyclic collection cannot execute"
    );
    let map: BTreeMap<_, _> = all
        .iter()
        .map(|n| Ok((id(n)?.to_owned(), n)))
        .collect::<Result<_>>()?;
    let mut states = BTreeMap::new();
    let mut committed = BTreeMap::new();
    let mut failures = Vec::new();
    let batch = input["batch_size"].as_u64().unwrap_or(5);
    ensure!((1..=100).contains(&batch), "invalid batch size");
    let mut skipped = 0usize;
    let mut count = 0;
    for cid in strings(&sequence["full_prerequisite_order"]) {
        let n = map[&cid];
        let deps = prerequisites(n);
        if deps
            .iter()
            .any(|d| map.contains_key(d) && states.get(d).is_none_or(|s| *s != "COMMITTED"))
        {
            states.insert(cid.clone(), "DEFERRED");
            continue;
        }
        let result: Result<Value> = (|| {
            let relative = Path::new(path(n)?);
            let target = safe_path(vault_root, relative, false)?;
            let content = n["content"]
                .as_str()
                .context("authored note content required")?;
            validate_properties(content)?;
            let dependency_hashes: BTreeMap<_, _> = deps
                .iter()
                .filter_map(|id| {
                    committed
                        .get(id)
                        .map(|v: &Value| (id.clone(), v["sha256"].clone()))
                })
                .collect();
            let inputs = hash(&serde_json::to_vec(
                &json!({"content":content,"dependencies":dependency_hashes,"configuration":input["configuration"]}),
            )?);
            let previous = &previous["committed"][&cid];
            if target.exists() {
                let actual = hash(&fs::read(&target)?);
                if previous["sha256"] == actual && previous["input_hash"] == inputs {
                    skipped += 1;
                    return Ok(
                        json!({"path":relative,"sha256":actual,"input_hash":inputs,"validated":true}),
                    );
                }
            }
            let expected = n["expected_hash"].as_str().or(previous["sha256"].as_str());
            vault::write_checked(vault_root, relative, content, expected)?;
            Ok(
                json!({"path":relative,"sha256":hash(content.as_bytes()),"input_hash":inputs,"dependency_hashes":dependency_hashes,"validated":true}),
            )
        })();
        match result {
            Ok(record) => {
                states.insert(cid.clone(), "COMMITTED");
                committed.insert(cid, record);
            }
            Err(e) => {
                states.insert(cid.clone(), "FAILED");
                failures.push(json!({"concept_id":cid,"error":format!("{e:#}")}));
            }
        }
        count += 1;
        if count % batch as usize == 0 {
            atomic_json(
                &checkpoint,
                &json!({"collection_id":collection,"committed":committed,"note_states":states,"failed":failures,"complete":false}),
            )?;
        }
    }
    let complete = states.len() == all.len() && states.values().all(|s| *s == "COMMITTED");
    let manifest = json!({"collection_id":collection,"complete":complete,"committed":committed,"note_states":states,"failed":failures,"skipped_already_committed":skipped,"checkpoint_path":checkpoint_relative});
    atomic_json(&checkpoint, &manifest)?;
    Ok(manifest)
}
pub fn competency_coverage(input: &Value) -> Result<Value> {
    let note = input["note_text"].as_str().context("note_text required")?;
    let (_, body) = vault::frontmatter(note)?;
    let lower = body.to_lowercase();
    let mut supported = Vec::new();
    let mut unsupported = Vec::new();
    let outcomes = input["outcomes"].as_array().context("outcomes required")?;
    for o in outcomes {
        let signals = strings(&o["expected_signals"]);
        let excerpt = o["expected_excerpt"].as_str();
        let matched = excerpt.is_some_and(|s| !s.is_empty() && body.contains(s))
            || signals
                .iter()
                .any(|s| !s.is_empty() && lower.contains(&s.to_lowercase()));
        if matched {
            supported.push(o["id"].clone());
        } else {
            unsupported.push(json!({"outcome_id":o["id"],"category":o["category"],"target":o["target"],"missing_signals":signals}));
        }
    }
    Ok(
        json!({"covered":unsupported.is_empty(),"total_outcomes":outcomes.len(),"supported_outcomes":supported,"unsupported_outcomes":unsupported,"claims_learner_mastery":false,"semantic_review_required":true}),
    )
}
pub fn collection_change_impact(input: &Value) -> Result<Value> {
    let all = nodes(input)?;
    let changed = input["changed_concept_id"]
        .as_str()
        .context("changed id required")?;
    let semantic = detect_semantic_changes(input)?;
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in all {
        let cid = id(n)?;
        for dep in prerequisites(n)
            .into_iter()
            .chain(strings(&n["related_ids"]))
        {
            graph.entry(dep).or_default().push(cid.into());
        }
        if let Some(parent) = n["parent_id"].as_str() {
            graph.entry(cid.into()).or_default().push(parent.into());
        }
    }
    for synthesis in input["synthesis_notes"].as_array().into_iter().flatten() {
        for cid in strings(&synthesis["linked_concepts"]) {
            graph
                .entry(cid)
                .or_default()
                .push(format!("synthesis:{}", title(synthesis)));
        }
    }
    let formatting = matches!(semantic["change_type"].as_str(), Some("none" | "cosmetic"));
    let impact = compute_change_impact(
        &json!({"changed_nodes":[changed],"dependency_graph":graph,"manually_edited":input["manually_edited"]}),
    )?;
    let targets = if formatting {
        vec![]
    } else {
        strings(&impact["impacted"])
    };
    Ok(
        json!({"changed_concept_id":changed,"change_type":if formatting{"formatting"}else{semantic["change_type"].as_str().unwrap_or("editorial")},"semantic_diff":semantic,"requires_downstream_review":!targets.is_empty(),"review_queue":targets.iter().map(|id|json!({"target_id":id,"status":"needs_review","manual_conflict":strings(&input["manually_edited"]).contains(id)})).collect::<Vec<_>>(),"manual_edit_conflicts":impact["manual_edit_conflicts"]}),
    )
}
pub fn reader_feedback(input: &Value) -> Result<Value> {
    let vault_root = Path::new(input["vault"].as_str().context("vault required")?);
    let feedback = input["feedback"]
        .as_str()
        .context("feedback required")?
        .to_lowercase();
    let category = if feedback.contains("prerequisite") {
        "missing_prerequisite"
    } else if feedback.contains("formula") {
        "unclear_formula"
    } else if feedback.contains("example") {
        "missing_example"
    } else if feedback.contains("difficult") {
        "too_difficult"
    } else {
        "confusing_explanation"
    };
    let paths = if let Some(p) = input["path"].as_str() {
        vec![p.to_owned()]
    } else {
        vault::index(vault_root, None)?
            .into_iter()
            .filter(|n| feedback.contains(&n.title.to_lowercase()))
            .map(|n| n.file)
            .collect()
    };
    if paths.len() != 1 {
        return Ok(
            json!({"status":if paths.is_empty(){"note_not_found"}else{"ambiguous_note_reference"},"candidates":paths,"applied":false,"claims_learner_mastery":false}),
        );
    }
    let relative = Path::new(&paths[0]);
    let target = safe_path(vault_root, relative, true)?;
    let original = fs::read_to_string(target)?;
    let before = input["find_text"].as_str();
    let replacement = input["replace_text"].as_str();
    if before.is_none() || replacement.is_none() {
        return Ok(
            json!({"status":"authoring_required","feedback_category":category,"target_note":relative,"source_policy":input["source_policy"],"required_authoring":"Supply exact original passage and an authored replacement supported by existing sources","applied":false,"claims_learner_mastery":false}),
        );
    }
    let before = before.unwrap();
    let replacement = replacement.unwrap();
    ensure!(
        !before.is_empty() && original.matches(before).count() == 1,
        "repair target must match exactly once"
    );
    if input["source_policy"] == "strict" && input["introduces_external_facts"] == true {
        bail!("strict source policy forbids external repair facts");
    }
    let proposed = original.replacen(before, replacement, 1);
    validate_properties(&proposed)?;
    let applied = input["apply"] == true;
    if applied {
        let expected = input["expected_hash"]
            .as_str()
            .context("repair requires reviewed original hash")?;
        vault::write_checked(vault_root, relative, &proposed, Some(expected))?;
    }
    Ok(
        json!({"status":if applied{"applied"}else{"proposal_ready"},"applied":applied,"feedback_category":category,"target_note":relative,"original_sha256":hash(original.as_bytes()),"proposed_sha256":hash(proposed.as_bytes()),"proposed_content":proposed,"claims_learner_mastery":false}),
    )
}
pub fn notation_consistency(input: &Value) -> Result<Value> {
    let entries = input["entries"]
        .as_array()
        .context("notation entries required")?;
    let mut map = BTreeMap::new();
    let mut scopes = BTreeMap::<(String, String), Vec<String>>::new();
    let mut conflicts = Vec::new();
    let mut mappings = Vec::new();
    for entry in entries {
        let cid = entry["concept_id"]
            .as_str()
            .context("notation concept id required")?;
        let symbol = entry["preferred_symbol"]
            .as_str()
            .context("preferred symbol required")?;
        let scope = entry["scope"]
            .as_str()
            .unwrap_or("general")
            .trim()
            .to_lowercase();
        ensure!(
            map.insert(cid, entry).is_none(),
            "duplicate notation concept"
        );
        scopes
            .entry((scope, symbol.trim().into()))
            .or_default()
            .push(cid.into());
    }
    for ((scope, symbol), ids) in scopes {
        if ids.len() > 1 {
            conflicts.push(json!({"type":"same_scope_symbol_collision","scope":scope,"symbol":symbol,"concept_ids":ids}));
        }
    }
    for usage in input["note_usages"].as_array().into_iter().flatten() {
        let cid = usage["concept_id"]
            .as_str()
            .context("usage concept required")?;
        let entry = map.get(cid).context("unregistered notation usage")?;
        let symbol = usage["symbol"].as_str().unwrap_or("").trim();
        let preferred = entry["preferred_symbol"].as_str().unwrap();
        if !symbol.is_empty() && symbol != preferred {
            if strings(&entry["accepted_symbols"]).contains(&symbol.into()) {
                if usage["mapping_explained"] == true || usage["source_specific"] == true {
                    mappings.push(json!({"concept_id":cid,"source_symbol":symbol,"preferred_symbol":preferred,"preserve_source_notation":true}));
                } else {
                    conflicts.push(
                        json!({"type":"unexplained_symbol_alias","concept_id":cid,"symbol":symbol}),
                    );
                }
            } else {
                conflicts
                    .push(json!({"type":"incompatible_symbol","concept_id":cid,"symbol":symbol}));
            }
        }
        let units = usage["units"].as_str().unwrap_or("").trim();
        let expected = entry["units"].as_str().unwrap_or("").trim();
        if !units.is_empty() && !expected.is_empty() && !units.eq_ignore_ascii_case(expected) {
            conflicts.push(json!({"type":"unit_inconsistency","concept_id":cid,"expected_units":expected,"observed_units":units}));
        }
    }
    Ok(
        json!({"consistent":conflicts.is_empty(),"registered_count":entries.len(),"conflicts":conflicts,"mappings_explained":mappings}),
    )
}
pub fn evaluate_granularity(input: &Value) -> Result<Value> {
    let cap = input["max_revisions"].as_u64().unwrap_or(2);
    if input["current_revision_count"].as_u64().unwrap_or(0) >= cap {
        return Ok(json!({"decisions":[],"revision_cap_reached":true}));
    }
    let all = nodes(input)?;
    let depth = input["depth"].as_str().unwrap_or("standard");
    let committed: BTreeSet<_> = strings(&input["committed_paths"]).into_iter().collect();
    let mut decisions = Vec::new();
    for n in all {
        let subtopics = strings(&n["subtopics"]);
        let split = subtopics.len() > 1 && ["comprehensive", "exhaustive"].contains(&depth);
        decisions.push(json!({"action":if split{"split"}else{"retain"},"concept_id":id(n)?,"title":title(n),"proposed_children":subtopics,"requires_expected_hash":split&&path(n).ok().is_some_and(|p|committed.contains(p)),"reason":if split{"Explicit independently teachable subtopics warrant separate chapters"}else{"Coherent current concept boundary"}}));
    }
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            if a["parent_id"] != b["parent_id"] {
                continue;
            }
            let a_words: BTreeSet<_> = normalize(title(a))
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            let b_words: BTreeSet<_> = normalize(title(b))
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            let union = a_words.union(&b_words).count();
            if union > 0 && a_words.intersection(&b_words).count() as f64 / union as f64 >= 0.8 {
                decisions.push(json!({"action":"merge","concept_ids":[id(a)?,id(b)?],"reason":"High conceptual overlap among sibling titles; author must verify semantic equivalence","requires_semantic_review":true}));
            }
        }
    }
    Ok(json!({"decisions":decisions,"revision_cap_reached":false}))
}
pub fn apply_granularity_split(input: &Value) -> Result<Value> {
    let mut plan = input["plan"].clone();
    let cid = input["target_concept_id"]
        .as_str()
        .context("target concept required")?;
    let children = strings(&input["child_titles"]);
    ensure!(
        children.len() >= 2 && children.len() <= 10,
        "split requires 2..10 authored child titles"
    );
    let mut all = nodes(&plan)?.clone();
    let index = all
        .iter()
        .position(|n| id(n).ok() == Some(cid))
        .context("unknown split target")?;
    let target = all[index].clone();
    let mut seen: BTreeSet<_> = all.iter().map(|n| id(n).unwrap_or("").to_owned()).collect();
    let mut child_ids = Vec::new();
    for title in children {
        ensure!(!title.trim().is_empty(), "empty split title");
        let child_id = format!("{cid}.{}", crate::knowledge::slug(&title));
        ensure!(
            seen.insert(child_id.clone()),
            "split child identity collision"
        );
        let mut child = target.clone();
        child["concept_id"] = json!(child_id);
        child["id"] = json!(child_id);
        child["title"] = json!(title);
        child["parent_id"] = json!(cid);
        child["child_ids"] = json!([]);
        child["note_role"] = json!("child");
        child["content"] = Value::Null;
        let original = Path::new(path(&target)?);
        child["proposed_path"] = json!(
            original
                .parent()
                .unwrap_or(Path::new(""))
                .join(format!(
                    "{}.md",
                    title.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-")
                ))
                .to_string_lossy()
                .replace('\\', "/")
        );
        child_ids.push(child_id);
        all.push(child);
    }
    all[index]["note_role"] = json!("group_index");
    all[index]["child_ids"] = json!(child_ids);
    all[index]["content"] = Value::Null;
    if plan["nodes"].is_array() {
        plan["nodes"] = json!(all);
    } else {
        plan["concepts"] = json!(all);
    }
    Ok(
        json!({"plan":plan,"split_target":cid,"new_child_ids":child_ids,"authoring_required":true,"existing_files_modified":false}),
    )
}
/// General synthesis uses supplied concept relationships and authored steps,
/// replacing the legacy topic-specific stock narratives.
pub fn plan_synthesis(input: &Value) -> Result<Value> {
    let all = nodes(input)?;
    let ids: BTreeSet<_> = all
        .iter()
        .map(|n| id(n).map(str::to_owned))
        .collect::<Result<_>>()?;
    let mut candidates = Vec::new();
    for group in input["groups"].as_array().into_iter().flatten() {
        let concepts = strings(&group["linked_concepts"]);
        ensure!(
            concepts.len() >= 2 && concepts.iter().all(|id| ids.contains(id)),
            "synthesis needs two known concepts"
        );
        let title = group["title"]
            .as_str()
            .context("synthesis title required")?;
        let steps = strings(&group["flow_steps"]);
        let content = if steps.is_empty() {
            Value::Null
        } else {
            json!(format!(
                "# {title}\n\n## Integrated Conceptual Mechanism\n\n{}\n\n## Connected Canonical Notes\n\n{}\n",
                steps
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("{}. {s}", i + 1))
                    .collect::<Vec<_>>()
                    .join("\n"),
                concepts
                    .iter()
                    .map(|id| {
                        let n = all.iter().find(|n| self::id(n).ok() == Some(id)).unwrap();
                        format!("- [[{}]]", self::title(n))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        };
        candidates.push(json!({"title":title,"master":"M7","note_type":"synthesis-revision","linked_concepts":concepts,"content_body":content,"status":if steps.is_empty(){"authoring_required"}else{"authored_draft"},"source_policy":input["source_policy"],"requires_source_and_semantic_review":true}));
    }
    Ok(json!({"synthesis_notes":candidates}))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn note() -> &'static str {
        "---\ntopic: Energy\ntags: [topic/energy]\nmaster_topic: Physics\ndomain: Physics\nnote_type: deep-notes\n---\n# Energy\nA conserved quantity.\n"
    }
    #[test]
    fn checkpoints_resume_and_preserve_manual_edits() {
        let dir = tempfile::tempdir().unwrap();
        let input = json!({"vault":dir.path(),"collection_id":"test","nodes":[{"id":"a","title":"Energy","path":"Energy.md","content":note()}]});
        let first = run_checkpoint(&input).unwrap();
        assert_eq!(first["complete"], true);
        assert_eq!(
            run_checkpoint(&input).unwrap()["skipped_already_committed"],
            1
        );
        fs::write(dir.path().join("Energy.md"), format!("{}Manual", note())).unwrap();
        assert_eq!(run_checkpoint(&input).unwrap()["complete"], false);
        assert!(
            fs::read_to_string(dir.path().join("Energy.md"))
                .unwrap()
                .ends_with("Manual")
        );
    }
    #[test]
    fn cycle_blocks_generation() {
        let dir = tempfile::tempdir().unwrap();
        assert!(run_checkpoint(&json!({"vault":dir.path(),"collection_id":"cycle","nodes":[{"id":"a","title":"A","prerequisites":["b"],"path":"a.md","content":note()},{"id":"b","title":"B","prerequisites":["a"],"path":"b.md","content":note()}]})).is_err());
    }
    #[test]
    fn self_reported_knowledge_skips_without_removing_link() {
        let r=learning_paths(&json!({"nodes":[{"id":"a","title":"Energy","path":"a.md"},{"id":"b","title":"Work","path":"b.md","prerequisites":["a"]}],"known_concepts":["Energy"]})).unwrap();
        assert_eq!(r["active_reading_order"], json!(["b"]));
        assert_eq!(r["steps"][0]["keep_prerequisite_link"], true);
    }
}
