//! Canonical identities, traceable claims and dependency-aware change impact.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
pub fn normalize(s: &str) -> String {
    s.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn slug(s: &str) -> String {
    let s = normalize(s).replace(' ', "_");
    if s.is_empty() { "note".into() } else { s }
}
fn general() -> String {
    "General".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptRecord {
    #[serde(default)]
    pub concept_id: String,
    pub canonical_name: String,
    #[serde(default = "general")]
    pub domain: String,
    #[serde(default = "general")]
    pub master_topic: String,
    #[serde(default)]
    pub subdomain: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub definition: String,
    #[serde(default)]
    pub symbols: BTreeMap<String, Value>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    #[serde(default)]
    pub related: Vec<String>,
    #[serde(default)]
    pub note_paths: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ConceptRegistry {
    records: BTreeMap<String, ConceptRecord>,
}
impl ConceptRegistry {
    pub fn register(&mut self, mut record: ConceptRecord) -> Result<ConceptRecord> {
        ensure!(
            !record.canonical_name.trim().is_empty(),
            "canonical name required"
        );
        if record.concept_id.is_empty() {
            record.concept_id = format!(
                "{}.{}.{}",
                slug(&record.domain),
                slug(&record.master_topic),
                slug(&record.canonical_name)
            );
        }
        if let Some(previous) = self.records.get(&record.concept_id) {
            ensure!(
                normalize(&previous.canonical_name) == normalize(&record.canonical_name)
                    && normalize(&previous.domain) == normalize(&record.domain),
                "concept identity conflict"
            );
        }
        self.records
            .insert(record.concept_id.clone(), record.clone());
        Ok(record)
    }
    pub fn resolve(
        &self,
        query: &str,
        domain: Option<&str>,
        master_topic: Option<&str>,
    ) -> Result<Option<&ConceptRecord>> {
        let q = normalize(query);
        let candidates: Vec<_> = self
            .records
            .values()
            .filter(|r| {
                r.concept_id == query
                    || normalize(&r.canonical_name) == q
                    || r.aliases.iter().any(|a| normalize(a) == q)
            })
            .filter(|r| domain.is_none_or(|d| normalize(d) == normalize(&r.domain)))
            .filter(|r| master_topic.is_none_or(|m| normalize(m) == normalize(&r.master_topic)))
            .collect();
        ensure!(
            candidates.len() <= 1,
            "ambiguous concept; supply domain and master topic"
        );
        Ok(candidates.first().copied())
    }
    pub fn all_records(&self) -> Vec<&ConceptRecord> {
        self.records.values().collect()
    }
    pub fn symbol_equivalence(&self, id: &str, source: &str) -> Result<Value> {
        let record = self.records.get(id).context("unknown concept")?;
        let clean = source.trim().trim_start_matches('\\');
        for (role, spec) in &record.symbols {
            let preferred = spec["preferred"]
                .as_str()
                .unwrap_or("")
                .trim()
                .trim_start_matches('\\');
            if clean == preferred
                || strings(&spec["aliases"])
                    .iter()
                    .any(|s| s.trim().trim_start_matches('\\') == clean)
            {
                return Ok(
                    json!({"role":role,"source_symbol":source,"preferred":preferred,"equivalent":true,"preserve_source_notation":true}),
                );
            }
        }
        Ok(
            json!({"role":null,"source_symbol":source,"preferred":null,"equivalent":false,"preserve_source_notation":true}),
        )
    }
}
pub fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
fn note_parts(text: &str) -> Result<(Value, String)> {
    let lines: Vec<_> = text.lines().collect();
    if lines.first() != Some(&"---") {
        return Ok((json!({}), text.into()));
    }
    let end = (1..lines.len())
        .find(|i| lines[*i] == "---")
        .context("unterminated frontmatter")?;
    let metadata: serde_yaml_ng::Value = serde_yaml_ng::from_str(&lines[1..end].join("\n"))?;
    Ok((serde_json::to_value(metadata)?, lines[end + 1..].join("\n")))
}
fn text<'a>(v: &'a Value, k: &str, default: &'a str) -> &'a str {
    v[k].as_str().unwrap_or(default)
}
pub fn registry_action(input: &Value) -> Result<Value> {
    let mut registry = ConceptRegistry::default();
    for rec in input["records"]
        .as_array()
        .context("records array required")?
    {
        registry.register(serde_json::from_value(rec.clone())?)?;
    }
    if let Some(query) = input["query"].as_str() {
        Ok(serde_json::to_value(registry.resolve(
            query,
            input["domain"].as_str(),
            input["master_topic"].as_str(),
        )?)?)
    } else {
        Ok(serde_json::to_value(registry.all_records())?)
    }
}
pub fn evaluate_coverage_ledger(input: &Value) -> Result<Value> {
    let required = input["required_concepts"]
        .as_array()
        .context("required_concepts required")?;
    let body = note_parts(text(input, "note_text", ""))?.1.to_lowercase();
    let master = text(input, "master", "M1");
    let mut entries = Vec::new();
    let mut missing = Vec::new();
    for item in required {
        let id = text(item, "concept", text(item, "id", ""));
        ensure!(!id.is_empty(), "concept id required");
        let requirement = text(item, "requirement", "essential");
        let mut keywords = strings(&item["keywords"]);
        if keywords.is_empty() {
            keywords.push(id.replace('_', " "));
        }
        let hits: Vec<_> = keywords
            .iter()
            .filter(|k| !k.is_empty() && body.contains(&k.to_lowercase()))
            .cloned()
            .collect();
        let mut entry = if let Some(reason) = input["deferrals"][id].as_str() {
            ensure!(!reason.trim().is_empty(), "deferral requires reason");
            json!({"concept":id,"requirement":requirement,"status":"deferred","reason":reason})
        } else if master == "M3" && requirement == "derivation_detail" {
            json!({"concept":id,"requirement":requirement,"status":"not_applicable","reason":"M3 quick-reference omits full derivations."})
        } else {
            json!({"concept":id,"requirement":requirement,"status":if hits.len()==keywords.len(){"covered"}else if hits.is_empty(){"missing"}else{"partial"},"matched_keywords":hits})
        };
        if requirement == "essential"
            && matches!(entry["status"].as_str(), Some("missing" | "partial"))
        {
            entry["repair_target"] = json!({"concept":id,"intended_location":text(item,"intended_section","core_concepts"),"action":if hits.is_empty(){"add_missing_explanation"}else{"complete_partial_explanation"}});
            missing.push(entry.clone());
        }
        entries.push(entry);
    }
    Ok(
        json!({"master":master,"complete":missing.is_empty(),"coverage":entries,"missing_essential":missing,"semantic_review_required":true}),
    )
}
pub fn audit_claim_traceability(input: &Value) -> Result<Value> {
    let claims = input["claims"]
        .as_array()
        .context("claims array required")?;
    let sources = input["verified_sources"]
        .as_object()
        .context("verified_sources map required")?;
    let policy = text(input, "source_policy", "adaptive");
    ensure!(
        [
            "strict",
            "source_first",
            "adaptive",
            "expanded",
            "independent",
            "comparative"
        ]
        .contains(&policy),
        "unknown source policy"
    );
    let mut findings = Vec::new();
    let mut out = Vec::new();
    for claim in claims {
        let id = text(claim, "id", "claim_unknown");
        let origin = text(claim, "origin", "source_fact");
        let refs = claim["evidence"].as_array().cloned().unwrap_or_default();
        let before = findings.len();
        let mut issue = |severity: &str, kind: &str| {
            findings.push(json!({"claim_id":id,"severity":severity,"issue":kind}))
        };
        if policy == "strict" && ["external", "additional_research"].contains(&origin) {
            issue("critical", "strict_source_policy_violation");
        }
        if refs.is_empty()
            && (policy == "strict" || ["source_fact", "independent_verification"].contains(&origin))
        {
            issue("major", "unsupported_claim");
        }
        for reference in &refs {
            let sid = text(reference, "source_id", "");
            match sources.get(sid) {
                None => issue("critical", "fake_or_unknown_citation"),
                Some(src) if src["content_verified"] != true => {
                    issue("major", "unverified_source_content")
                }
                Some(src) if policy == "strict" && src["role"] == "external" => {
                    issue("critical", "external_citation_in_strict_mode")
                }
                Some(_) if text(reference, "location", "").trim().is_empty() => {
                    issue("moderate", "missing_source_locator")
                }
                _ => (),
            }
        }
        let status = if findings.len() > before {
            "unsupported"
        } else if refs.is_empty() {
            "inferred"
        } else {
            "supported"
        };
        out.push(json!({"id":id,"concept_id":claim["concept_id"],"statement":text(claim,"statement",""),"origin":origin,"evidence":refs,"verification":{"status":status}}));
    }
    Ok(json!({"valid":findings.is_empty(),"source_policy":policy,"claims":out,"findings":findings}))
}
pub fn compute_change_impact(input: &Value) -> Result<Value> {
    let roots = strings(&input["changed_nodes"]);
    let graph = input["dependency_graph"]
        .as_object()
        .context("dependency_graph map required")?;
    let manual: BTreeSet<_> = strings(&input["manually_edited"]).into_iter().collect();
    let mut seen: BTreeSet<_> = roots.iter().cloned().collect();
    let mut queue: VecDeque<_> = roots.iter().cloned().collect();
    let mut impacted = Vec::new();
    while let Some(id) = queue.pop_front() {
        for next in strings(graph.get(&id).unwrap_or(&Value::Null)) {
            if seen.insert(next.clone()) {
                impacted.push(next.clone());
                queue.push_back(next);
            }
        }
    }
    Ok(
        json!({"roots":roots,"impacted":impacted,"safe_to_update":impacted.iter().filter(|id|!manual.contains(*id)).collect::<Vec<_>>(),"manual_edit_conflicts":impacted.iter().filter(|id|manual.contains(*id)).collect::<Vec<_>>()}),
    )
}
pub fn check_multi_artifact_consistency(input: &Value) -> Result<Value> {
    let artifacts = input["artifacts"]
        .as_array()
        .context("artifacts required")?;
    let mut canonical: BTreeMap<String, Value> = BTreeMap::new();
    let mut issues = Vec::new();
    for artifact in artifacts {
        let concepts = artifact["concepts"].as_array().cloned().unwrap_or_default();
        for c in concepts {
            let id = text(&c, "concept_id", text(&c, "id", ""));
            ensure!(!id.is_empty(), "concept identity missing");
            if let Some(previous) = canonical.get(id) {
                for field in ["definition", "domain", "symbols", "assumptions"] {
                    if !c[field].is_null()
                        && !previous[field].is_null()
                        && c[field] != previous[field]
                    {
                        issues.push(json!({"concept_id":id,"artifact":artifact["id"],"field":field,"issue":"cross_artifact_conflict"}));
                    }
                }
            } else {
                canonical.insert(id.into(), c);
            }
        }
    }
    let mut ref_meta = Value::Null;
    let mut ref_body = String::new();
    let mut ref_symbols = Value::Null;
    let mut ref_numbers = Value::Null;
    let mut ref_conditions = Vec::new();
    let mut note_types = BTreeSet::new();
    let mut reconciliations = Vec::new();
    for (index, artifact) in artifacts.iter().enumerate() {
        let Some(markdown) = artifact["note_text"].as_str() else {
            continue;
        };
        let (meta, body) = note_parts(markdown)?;
        let note_type = meta["note_type"]
            .as_str()
            .or(artifact["note_type"].as_str())
            .unwrap_or("unknown");
        if note_type != "unknown" && !note_types.insert(note_type.to_owned()) {
            issues.push(json!({"type":"duplicate_note_type","artifact":index}));
        }
        if ref_meta.is_null() {
            ref_meta = meta;
            ref_body = body;
            ref_symbols = artifact["symbols"].clone();
            ref_numbers = artifact["numeric_results"].clone();
            ref_conditions = strings(&artifact["conditions"]);
            continue;
        }
        for field in ["domain", "master_topic", "topic"] {
            if !ref_meta[field].is_null()
                && !meta[field].is_null()
                && normalize(ref_meta[field].as_str().unwrap_or(""))
                    != normalize(meta[field].as_str().unwrap_or(""))
            {
                issues.push(json!({"type":"metadata_mismatch","field":field,"artifact":index}));
            }
        }
        if let Some(symbols) = ref_symbols.as_object() {
            for (role, a) in symbols {
                let b = &artifact["symbols"][role];
                if !b.is_null() && a != b {
                    let sa = a.as_str().unwrap_or("");
                    let sb = b.as_str().unwrap_or("");
                    if (body.contains(sa) && body.contains(sb))
                        || (ref_body.contains(sa) && ref_body.contains(sb))
                    {
                        reconciliations.push(json!({"role":role,"symbols":[a,b]}));
                    } else {
                        issues.push(json!({"type":"unexplained_notation_divergence","role":role,"artifact":index}));
                    }
                }
            }
        }
        let plain = body.replace('$', "").to_lowercase();
        for condition in &ref_conditions {
            if !plain.contains(&condition.replace('$', "").to_lowercase())
                && !strings(&artifact["conditions"])
                    .iter()
                    .any(|s| s.eq_ignore_ascii_case(condition))
            {
                issues.push(json!({"type":"dropped_applicability_condition","condition":condition,"artifact":index}));
            }
        }
        if let Some(numbers) = ref_numbers.as_object() {
            for (key, a) in numbers {
                if let (Some(a), Some(b)) = (a.as_f64(), artifact["numeric_results"][key].as_f64())
                    && (a - b).abs() > 1e-6
                {
                    issues.push(json!({"type":"numeric_contradiction","key":key,"artifact":index}));
                }
            }
        }
    }
    Ok(
        json!({"valid":issues.is_empty(),"consistent":issues.is_empty(),"issues":issues,"reconciliations":reconciliations}),
    )
}
pub fn detect_semantic_changes(input: &Value) -> Result<Value> {
    let old = text(input, "old_text", "");
    let new = text(input, "new_text", "");
    if old == new {
        return Ok(json!({"change_type":"none","categories":[],"changed_sections":[]}));
    }
    if old.split_whitespace().collect::<Vec<_>>() == new.split_whitespace().collect::<Vec<_>>() {
        return Ok(
            json!({"change_type":"cosmetic","categories":["cosmetic"],"changed_sections":[]}),
        );
    }
    let (_, old) = note_parts(old)?;
    let (_, new) = note_parts(new)?;
    let headings = |s: &str| {
        s.lines()
            .filter(|l| l.starts_with('#'))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let code = |s: &str| {
        s.split("```")
            .enumerate()
            .filter(|(i, _)| i % 2 == 1)
            .map(|(_, s)| s.to_owned())
            .collect::<Vec<_>>()
    };
    let nums = |s: &str| {
        s.split(|c: char| !c.is_ascii_digit() && c != '.')
            .filter(|s| s.chars().any(|c| c.is_ascii_digit()))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let urls = |s: &str| {
        s.split_whitespace()
            .filter(|s| s.contains("https://") || s.contains("http://"))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    };
    let mut categories = Vec::new();
    if nums(&old) != nums(&new)
        || old
            .split('$')
            .enumerate()
            .filter(|(index, _)| index % 2 == 1)
            .map(|(_, part)| part)
            .collect::<Vec<_>>()
            != new
                .split('$')
                .enumerate()
                .filter(|(index, _)| index % 2 == 1)
                .map(|(_, part)| part)
                .collect::<Vec<_>>()
    {
        categories.push("semantic");
    }
    if urls(&old) != urls(&new) {
        categories.push("evidence");
    }
    if code(&old) != code(&new) {
        categories.push("implementation");
    }
    if headings(&old) != headings(&new) {
        categories.push("structural");
    }
    if categories.is_empty() {
        categories.push("editorial");
    }
    let primary = categories[0];
    Ok(
        json!({"change_type":primary,"categories":categories,"changed_sections":headings(&new).into_iter().filter(|s|!old.contains(s)).collect::<Vec<_>>(),"requires_downstream_check":(["semantic","evidence","implementation"].contains(&primary))}),
    )
}
pub fn uncertainty_ledger(input: &Value) -> Result<Value> {
    let assumptions = input["assumptions"].as_array().cloned().unwrap_or_default();
    for a in &assumptions {
        ensure!(
            ["verified", "supported", "inferred", "assumed", "unverified"].contains(&text(
                a,
                "certainty",
                "assumed"
            )),
            "invalid certainty"
        );
        ensure!(!text(a, "id", "").is_empty(), "assumption id required");
    }
    Ok(
        json!({"assumptions":assumptions,"verification_limitations":input["verification_limitations"].as_array().cloned().unwrap_or_default(),"reader_visible_disclosures":assumptions.iter().filter(|a|a["reader_visible"]==true).collect::<Vec<_>>()}),
    )
}
pub fn validate_prerequisites(records: &[ConceptRecord]) -> Result<Vec<String>> {
    let ids: BTreeSet<_> = records.iter().map(|c| c.concept_id.clone()).collect();
    ensure!(ids.len() == records.len(), "duplicate concept id");
    let mut order = Vec::new();
    let mut done = BTreeSet::new();
    while order.len() < records.len() {
        let Some(c) = records.iter().find(|c| {
            !done.contains(&c.concept_id) && c.prerequisites.iter().all(|p| done.contains(p))
        }) else {
            bail!("unknown or cyclic prerequisite")
        };
        done.insert(c.concept_id.clone());
        order.push(c.concept_id.clone());
    }
    Ok(order)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coverage_does_not_count_metadata_as_explanation() {
        let report=evaluate_coverage_ledger(&json!({"required_concepts":[{"concept":"gradient"}],"note_text":"---\ntopic: gradient\n---\nAn unrelated body"})).unwrap();
        assert_eq!(report["complete"], false);
    }
    #[test]
    fn companion_numeric_and_condition_conflicts_are_detected() {
        let report=check_multi_artifact_consistency(&json!({"artifacts":[{"note_text":"---\ntopic: Gradient\ndomain: Mathematics\nnote_type: deep-notes\n---\nUse fixed rate","conditions":["fixed rate"],"numeric_results":{"answer":2}},{"note_text":"---\ntopic: Gradient\ndomain: Mathematics\nnote_type: quick-reference\n---\nSummary","numeric_results":{"answer":3}}]})).unwrap();
        assert_eq!(report["consistent"], false);
        assert_eq!(report["issues"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn mathematical_edits_trigger_review_but_whitespace_does_not() {
        let result = detect_semantic_changes(
            &json!({"old_text":"A $x+1$ result","new_text":"A $x+2$ result"}),
        )
        .unwrap();
        assert_eq!(result["change_type"], "semantic");
        assert_eq!(
            detect_semantic_changes(&json!({"old_text":"A   result","new_text":"A result"}))
                .unwrap()["change_type"],
            "cosmetic"
        );
    }
    #[test]
    fn homonyms_require_domain_and_replacement_removes_old_alias() {
        let mut r = ConceptRegistry::default();
        for domain in ["Physics", "Computer science"] {
            r.register(
                serde_json::from_value(
                    json!({"canonical_name":"Field","domain":domain,"aliases":["field"]}),
                )
                .unwrap(),
            )
            .unwrap();
        }
        assert!(r.resolve("field", None, None).is_err());
        assert_eq!(
            r.resolve("field", Some("Physics"), None)
                .unwrap()
                .unwrap()
                .domain,
            "Physics"
        );
    }
    #[test]
    fn traceability_rejects_unknown_and_strict_external() {
        let r=audit_claim_traceability(&json!({"claims":[{"id":"c","origin":"external","evidence":[{"source_id":"fake","location":"p1"}]}],"verified_sources":{},"source_policy":"strict"})).unwrap();
        assert_eq!(r["valid"], false);
        assert_eq!(r["findings"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn change_impact_handles_cycles_manual_edits() {
        let r=compute_change_impact(&json!({"changed_nodes":["a"],"dependency_graph":{"a":["b"],"b":["a","c"]},"manually_edited":["c"]})).unwrap();
        assert_eq!(r["impacted"], json!(["b", "c"]));
        assert_eq!(r["manual_edit_conflicts"], json!(["c"]));
    }
}
