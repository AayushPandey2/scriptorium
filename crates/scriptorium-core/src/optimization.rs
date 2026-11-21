//! Selective context compilation and dependency-bound cache identities.
use crate::{cache::hash, knowledge::strings};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}
pub fn contains_secret(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "-----begin private key-----",
        "-----begin rsa private key-----",
        "-----begin openssh private key-----",
    ]
    .iter()
    .any(|p| lower.contains(p))
        || text.split_whitespace().any(|word| {
            let word = word.trim_matches(['\"', '\'', '=', ',', ':', '{', '}']);
            (word.starts_with("sk-") && word.len() > 24)
                || (word.starts_with("ghp_") && word.len() > 24)
                || (word.starts_with("AKIA") && word.len() >= 20)
        })
}
fn safe_payload(value: &Value) -> Result<()> {
    ensure!(
        !contains_secret(&serde_json::to_string(value)?),
        "context/cache payload contains a secret"
    );
    Ok(())
}
pub fn dependency_cache_key(input: &Value) -> Result<Value> {
    safe_payload(input)?;
    let scope = input["user_scope"]
        .as_str()
        .context("explicit user_scope required for cache isolation")?;
    ensure!(!scope.trim().is_empty(), "empty user scope");
    let dependencies = input["dependency_hashes"]
        .as_object()
        .context("dependency_hashes map required")?;
    for value in dependencies.values() {
        let digest = value.as_str().context("dependency digest must be string")?;
        ensure!(
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "dependency must have SHA-256 digest"
        );
    }
    let canonical = json!({"schema":1,"user_scope":scope,"layer":input["layer"],"version":input["version"],"source_hash":input["source_hash"],"dependency_hashes":dependencies,"parameters":input["parameters"]});
    Ok(json!({"key":hash(&serde_json::to_vec(&canonical)?),"identity":canonical}))
}
pub fn partition_context(input: &Value) -> Result<Value> {
    let kcp = input.get("kcp").unwrap_or(input);
    let plan = kcp.get("plan").unwrap_or(kcp);
    let target_id = input["target_concept_id"]
        .as_str()
        .context("target_concept_id required")?;
    let nodes = plan["nodes"]
        .as_array()
        .or(plan["concepts"].as_array())
        .context("plan nodes/concepts required")?;
    let id = |n: &Value| {
        n["concept_id"]
            .as_str()
            .or(n["id"].as_str())
            .unwrap_or("")
            .to_owned()
    };
    let map: BTreeMap<_, _> = nodes.iter().map(|n| (id(n), n)).collect();
    let target = map.get(target_id).context("unknown target concept")?;
    let parent = target["parent_id"].as_str().and_then(|id| map.get(id));
    let siblings: Vec<_> = nodes
        .iter()
        .filter(|n| n["parent_id"] == target["parent_id"] && id(n) != target_id)
        .map(|n| n["title"].clone())
        .collect();
    let refs = |keys: Vec<String>| {
        keys.iter().map(|id|json!({"concept_id":id,"title":map.get(id).map(|n|n["title"].clone()).unwrap_or(json!(id))})).collect::<Vec<_>>()
    };
    let prereq = if target["prerequisite_ids"].is_array() {
        strings(&target["prerequisite_ids"])
    } else {
        strings(&target["prerequisites"])
    };
    let mut note = json!({"concept_id":target_id,"title":target["title"],"proposed_path":target["proposed_path"],"template_plan":target["template_plan"],"depth_allocation":kcp["depth_allocation"]["allocations"][target_id],"direct_prerequisites":refs(prereq),"direct_related":refs(strings(&target["related_ids"])),"outcomes":kcp["competencies"][target_id],"source_bindings":target["source_bindings"]});
    if let Some(notation) = kcp["notation"].as_object() {
        note["notation"] = json!(
            notation
                .iter()
                .filter(|(_, v)| strings(&v["concept_ids"]).contains(&target_id.to_owned()))
                .collect::<BTreeMap<_, _>>()
        );
    }
    let result = json!({"global_context":{"collection_id":plan["collection_id"],"topic":plan.get("title").unwrap_or(&plan["topic"]),"configuration":kcp["configuration"],"scope":kcp["scope"]},"group_context":{"parent_id":target["parent_id"],"parent_title":parent.map(|p|p["title"].clone()).unwrap_or(plan["title"].clone()),"sibling_titles":siblings},"note_context":note});
    safe_payload(&result)?;
    let full = estimate_tokens(&serde_json::to_string(kcp)?);
    let selected = estimate_tokens(&serde_json::to_string(&result)?);
    let mut result = result;
    result["partitioned_tokens"] = json!(selected);
    result["full_kcp_tokens"] = json!(full);
    result["token_savings"] = json!(full.saturating_sub(selected));
    Ok(result)
}
/// Select declared module dependencies, not the full instruction repository.
pub fn compile_context(input: &Value) -> Result<Value> {
    let modules = input["modules"]
        .as_object()
        .context("modules registry map required")?;
    let roots = strings(&input["requested_modules"]);
    ensure!(!roots.is_empty(), "requested modules required");
    let mut order = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    fn visit(
        id: &str,
        modules: &serde_json::Map<String, Value>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if visited.contains(id) {
            return Ok(());
        }
        ensure!(
            visiting.insert(id.into()),
            "cyclic context module dependency"
        );
        let module = modules.get(id).context("unknown context module")?;
        for dep in strings(&module["dependencies"]) {
            visit(&dep, modules, visiting, visited, order)?;
        }
        visiting.remove(id);
        visited.insert(id.into());
        order.push(id.into());
        Ok(())
    }
    for root in roots {
        visit(&root, modules, &mut visiting, &mut visited, &mut order)?;
    }
    let slices:Vec<_>=order.iter().map(|id|json!({"id":id,"content":modules[id]["content"],"sha256":hash(serde_json::to_string(&modules[id]).unwrap().as_bytes())})).collect();
    let result = json!({"selected_modules":order,"context":slices,"preserved_validation_checks":["source_fidelity","citation_correctness","mathematical_correctness","code_correctness","explanation_quality","prerequisite_integrity","concept_coverage","terminology_consistency","learning_outcomes","assessment_correctness","link_integrity","yaml_correctness","collection_completeness","change_impact_consistency"]});
    safe_payload(&result)?;
    Ok(result)
}
pub fn source_segments(input: &Value) -> Result<Value> {
    let id = input["source_id"].as_str().context("source_id required")?;
    let scope = input["user_scope"]
        .as_str()
        .context("user_scope required")?;
    let text = input["text"].as_str().context("source text required")?;
    ensure!(
        !id.is_empty() && !scope.is_empty(),
        "source identity and scope required"
    );
    ensure!(
        !contains_secret(text),
        "private credentials cannot enter source cache"
    );
    let mut segments = Vec::new();
    let mut start = 1usize;
    let mut body = Vec::new();
    let mut title = "Introduction".to_owned();
    let flush = |segments: &mut Vec<Value>,
                 body: &mut Vec<String>,
                 start: usize,
                 end: usize,
                 title: &str| {
        let content = body.join("\n");
        if !content.trim().is_empty() {
            segments.push(json!({"segment_id":format!("{id}:{}",segments.len()+1),"title":title,"text":content,"start_line":start,"end_line":end,"sha256":hash(content.as_bytes())}));
        }
        body.clear();
    };
    let mut count = 0;
    for (index, line) in text.lines().enumerate() {
        count = index + 1;
        if line.starts_with('#') && !body.is_empty() {
            flush(&mut segments, &mut body, start, index, &title);
            start = index + 1;
        }
        if line.starts_with('#') {
            title = line.trim_start_matches('#').trim().to_owned();
        }
        body.push(line.to_owned());
    }
    flush(&mut segments, &mut body, start, count, &title);
    Ok(
        json!({"source_id":id,"user_scope":scope,"source_hash":hash(text.as_bytes()),"segments":segments,"total_segments":segments.len(),"coverage":segments.iter().map(|s|json!({"segment_id":s["segment_id"],"status":"unreviewed"})).collect::<Vec<_>>()}),
    )
}
pub fn retrieve_passages(input: &Value) -> Result<Value> {
    let store = &input["store"];
    ensure!(
        store["user_scope"] == input["user_scope"] && !input["user_scope"].is_null(),
        "source scope isolation violation"
    );
    let query = input["query"]
        .as_str()
        .context("query required")?
        .to_lowercase();
    let terms: BTreeSet<_> = query.split_whitespace().filter(|s| s.len() > 1).collect();
    let maximum = input["max_passages"].as_u64().unwrap_or(5);
    ensure!((1..=50).contains(&maximum), "max_passages must be 1..50");
    let mut ranked: Vec<_> = store["segments"]
        .as_array()
        .context("segments required")?
        .iter()
        .map(|s| {
            let text = s["text"].as_str().unwrap_or("").to_lowercase();
            (terms.iter().filter(|t| text.contains(**t)).count(), s)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1["start_line"].as_u64().cmp(&b.1["start_line"].as_u64()))
    });
    let passages: Vec<_> = ranked
        .into_iter()
        .take(maximum as usize)
        .map(|(score, s)| json!({"segment":s,"score":score}))
        .collect();
    Ok(
        json!({"source_id":store["source_id"],"source_hash":store["source_hash"],"passages":passages,"coverage_claim":"retrieved passages only; full-source coverage is not inferred"}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_closes_dependencies_without_sibling_bodies() {
        let r=compile_context(&json!({"requested_modules":["note"],"modules":{"note":{"content":"Note","dependencies":["shared"]},"shared":{"content":"Policy"},"unused":{"content":"large"}}})).unwrap();
        assert_eq!(r["selected_modules"], json!(["shared", "note"]));
    }
    #[test]
    fn scope_and_dependency_hashes_change_key() {
        let base = json!({"user_scope":"a","layer":"sources","version":1,"dependency_hashes":{"a":"a".repeat(64)}});
        let mut changed = base.clone();
        changed["user_scope"] = json!("b");
        assert_ne!(
            dependency_cache_key(&base).unwrap()["key"],
            dependency_cache_key(&changed).unwrap()["key"]
        );
    }
    #[test]
    fn segment_retrieval_tracks_locators_and_scope() {
        let store=source_segments(&json!({"source_id":"s","user_scope":"u","text":"# Energy\nConserved quantity\n# Entropy\nState function"})).unwrap();
        let r =
            retrieve_passages(&json!({"store":store,"user_scope":"u","query":"entropy"})).unwrap();
        assert_eq!(r["passages"][0]["segment"]["start_line"], 3);
        assert!(
            retrieve_passages(&json!({"store":store,"user_scope":"other","query":"entropy"}))
                .is_err()
        );
    }
}
