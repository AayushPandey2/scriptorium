//! Native contracts ported from the maintained instructional policy.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
pub fn registry() -> Value {
    serde_json::from_str(include_str!(
        "../../../references/workflows/notes-creation/instructional-registry.json"
    ))
    .expect("valid instructional registry")
}
fn names(value: &Value, allowed: &Value, field: &str) -> Result<Vec<String>> {
    let a = value
        .as_array()
        .with_context(|| format!("{field} must be list"))?;
    ensure!(!a.is_empty(), "{field} cannot be empty");
    let mut names = Vec::new();
    for v in a {
        let name = v.as_str().context("expected text name")?;
        ensure!(allowed.get(name).is_some(), "unknown {field}: {name}");
        if !names.iter().any(|s| s == name) {
            names.push(name.into());
        }
    }
    Ok(names)
}
fn supports(name: &str, c: &Value, depth: &BTreeMap<String, u64>, count: usize) -> bool {
    let types = strings(&c["types"]);
    let objectives = strings(&c["objectives"]);
    let has = |a: &[&str]| a.iter().any(|v| types.iter().any(|s| s == v));
    let goal = |a: &[&str]| a.iter().any(|v| objectives.iter().any(|s| s == v));
    match name {
        "motivation" => {
            strings(&registry()["subtemplates"][types.first().map(String::as_str).unwrap_or("")])
                .iter()
                .any(|s| s == "need")
                || c["prerequisites"].as_array().is_some_and(|a| {
                    a.iter().any(|d| {
                        ["explain", "foundation"].contains(&d["treatment"].as_str().unwrap_or(""))
                    })
                })
        }
        "intuition" => {
            depth["theory"] >= 2
                && has(&["conceptual", "mathematical", "scientific", "architectural"])
        }
        "definitions" => depth["theory"] >= 1 || depth["math"] >= 1 || depth["code"] >= 1,
        "mathematics" => {
            depth["math"] >= 2
                || (depth["math"] >= 1
                    && (has(&["mathematical"]) || goal(&["calculate", "derive", "prove"])))
        }
        "visualization" => {
            c["visualizations"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
                || (has(&["architectural"]) && depth["visual"] >= 2)
        }
        "coding" => {
            c["implementation"]["required"] == true
                || (depth["code"] >= 2 && (has(&["programming"]) || goal(&["implement", "debug"])))
        }
        "examples" => {
            depth["practice"] >= 2
                && (has(&[
                    "conceptual",
                    "mathematical",
                    "scientific",
                    "programming",
                    "algorithmic",
                    "procedural",
                ]) || goal(&["explain", "calculate", "apply", "implement", "trace"]))
        }
        "limitations" => {
            depth["theory"] >= 3
                && has(&[
                    "mathematical",
                    "algorithmic",
                    "architectural",
                    "scientific",
                    "comparative",
                    "procedural",
                ])
        }
        "assessment" => depth["assessment"] >= 2,
        "revision" => count > 1 || depth["theory"] >= 4,
        _ => false,
    }
}
pub fn compose_concept_sequence(concept: &Value) -> Result<Vec<Value>> {
    let reg = registry();
    let p = policy();
    let mut entries: BTreeMap<String, Value> = BTreeMap::new();
    let types = strings(&concept["types"]);
    let mut add = |stage: &str, module: &str, step: &str, kind: &str| {
        let e = entries
            .entry(stage.into())
            .or_insert(json!({"stage":stage,"module":module,"source_steps":[],"types":[]}));
        for (key, value) in [("source_steps", step), ("types", kind)] {
            let a = e[key].as_array_mut().unwrap();
            if !value.is_empty() && !a.iter().any(|v| v == value) {
                a.push(json!(value));
            }
        }
    };
    if let Some(deps) = concept["prerequisites"].as_array() {
        for d in deps {
            if ["reminder", "explain", "foundation"]
                .contains(&d["treatment"].as_str().unwrap_or(""))
            {
                add(
                    "prerequisite_preparation",
                    "definitions",
                    &format!(
                        "prerequisite:{}({})",
                        d["concept"].as_str().unwrap_or(""),
                        d["treatment"].as_str().unwrap_or("")
                    ),
                    "",
                );
            }
        }
    }
    for kind in &types {
        for raw in strings(&reg["subtemplates"][kind]) {
            let pair = &p["STAGE_CANONICAL"][&raw];
            let stage = pair[0].as_str().unwrap_or("mechanism_and_structure");
            let mut module = pair[1].as_str().unwrap_or("definitions");
            if stage == "derivation_and_proof" {
                module = if ["required", "supporting"]
                    .contains(&concept["modules"]["proof"]["status"].as_str().unwrap_or(""))
                {
                    "proof"
                } else if ["required", "supporting"].contains(
                    &concept["modules"]["derivation"]["status"]
                        .as_str()
                        .unwrap_or(""),
                ) {
                    "derivation"
                } else {
                    "mathematics"
                };
            }
            if concept["modules"][module]["status"] == "excluded" {
                if ["definitions_and_setup", "mechanism_and_structure"].contains(&stage)
                    && concept["depth"]["theory"].as_u64().unwrap_or(0) > 0
                {
                    module = "definitions";
                } else {
                    continue;
                }
            }
            add(stage, module, &raw, kind);
        }
    }
    for (module, stage) in [
        ("motivation", "motivation_and_problem"),
        ("history", "historical_context"),
        ("intuition", "intuition_and_model"),
        ("definitions", "definitions_and_setup"),
        ("visualization", "mechanism_and_structure"),
        ("mathematics", "mathematical_formulation"),
        ("derivation", "derivation_and_proof"),
        ("proof", "derivation_and_proof"),
        ("algorithm", "algorithmic_procedure"),
        ("coding", "implementation_and_code"),
        ("experiment", "experiment_and_evidence"),
        ("examples", "worked_application"),
        ("comparison", "comparison_and_distinctions"),
        ("limitations", "limitations_and_boundaries"),
        ("assessment", "assessment_and_practice"),
        ("revision", "revision_and_consolidation"),
    ] {
        if ["required", "supporting"]
            .contains(&concept["modules"][module]["status"].as_str().unwrap_or(""))
        {
            add(stage, module, module, "");
        }
    }
    Ok(strings(&p["STAGE_ORDER"])
        .into_iter()
        .filter_map(|stage| entries.remove(&stage))
        .collect())
}
pub fn normalize_plan(raw: &Value) -> Result<Value> {
    ensure!(raw.is_object(), "instructional plan must be object");
    let reg = registry();
    let p = policy();
    let audience = raw["audience"].as_str().unwrap_or("beginner");
    ensure!(
        ["beginner", "intermediate", "advanced"].contains(&audience),
        "invalid audience"
    );
    let source_policy = raw["source_policy"].as_str().unwrap_or("independent");
    ensure!(
        ["independent", "strict", "expanded", "comparative"].contains(&source_policy),
        "unknown source policy"
    );
    let mut concepts = raw["concepts"]
        .as_array()
        .context("concepts must be list")?
        .clone();
    ensure!(!concepts.is_empty(), "concepts cannot be empty");
    let total = concepts.len();
    let mut ids = BTreeSet::new();
    for c in &concepts {
        let id = c["id"].as_str().context("concept needs ID")?;
        ensure!(
            !id.is_empty()
                && id.chars().all(|c| c.is_alphanumeric() || "_-".contains(c))
                && ids.insert(id.to_string()),
            "concept IDs must be unique identifiers"
        );
    }
    let sources = raw["sources"].as_array().cloned().unwrap_or_default();
    if source_policy != "independent" {
        ensure!(
            !sources.is_empty(),
            "source grounded policy requires element inventory"
        );
    }
    let mut source_ids = BTreeSet::new();
    for s in &sources {
        let id = s["id"].as_str().context("source needs ID")?;
        ensure!(
            !id.is_empty() && source_ids.insert(id),
            "duplicate source ID"
        );
        for key in ["location", "kind"] {
            ensure!(
                s[key].as_str().is_some_and(|v| !v.trim().is_empty()),
                "source missing {key}"
            );
        }
        ensure!(
            ["full", "partial", "unavailable"].contains(&s["access"].as_str().unwrap_or(""))
                && s["required"].is_boolean(),
            "source needs access and required flag"
        );
        if s["required"] == true {
            ensure!(
                !strings(&s["concepts"]).is_empty()
                    && strings(&s["concepts"]).iter().all(|s| ids.contains(s)),
                "invalid source concept assignments"
            );
        } else {
            ensure!(
                s["reason"].as_str().is_some_and(|s| !s.is_empty()),
                "source omission requires reason"
            );
        }
    }
    let mut seen = BTreeSet::new();
    for c in &mut concepts {
        let id = c["id"].as_str().unwrap().to_string();
        ensure!(
            c["title"].as_str().is_some_and(|s| !s.trim().is_empty()),
            "concept needs title"
        );
        let reason = c["reason"]
            .as_str()
            .or_else(|| c["justification"].as_str())
            .context("concept needs reason")?
            .to_string();
        ensure!(!reason.trim().is_empty(), "empty concept reason");
        c["reason"] = json!(reason);
        let selected = c
            .get("types")
            .or_else(|| c.get("knowledge_types"))
            .or_else(|| c.get("subtemplates"))
            .context("concept needs knowledge types")?;
        let types = names(selected, &reg["subtemplates"], "types")?;
        let goals = names(&c["objectives"], &reg["objectives"], "objectives")?;
        c["types"] = json!(types);
        c["knowledge_types"] = c["types"].clone();
        c["subtemplates"] = c["types"].clone();
        c["objectives"] = json!(goals);
        let mut dependencies = Vec::new();
        let mut dep_ids = BTreeSet::new();
        for (field, treatment, kind) in [
            ("prerequisites", "explain", "required"),
            ("required_prerequisites", "explain", "required"),
            ("helpful_prerequisites", "reminder", "helpful"),
        ] {
            for d in c[field].as_array().cloned().unwrap_or_default() {
                let target = d
                    .as_str()
                    .or_else(|| d["concept"].as_str())
                    .context("prerequisite needs concept")?;
                let treatment = d["treatment"].as_str().unwrap_or(treatment);
                let kind = d["kind"].as_str().unwrap_or(kind);
                ensure!(
                    ["known", "reminder", "explain", "foundation"].contains(&treatment),
                    "invalid prerequisite treatment"
                );
                ensure!(
                    !ids.contains(target) || seen.contains(target),
                    "prerequisite must precede consumer"
                );
                if dep_ids.insert(target.to_string()) {
                    dependencies.push(json!({"concept":target,"treatment":treatment,"kind":kind}));
                }
            }
        }
        c["prerequisites"] = json!(dependencies);
        c["required_prerequisites"] = json!(
            dependencies
                .iter()
                .filter(|d| d["kind"] != "helpful")
                .map(|d| d["concept"].clone())
                .collect::<Vec<_>>()
        );
        c["helpful_prerequisites"] = json!(
            dependencies
                .iter()
                .filter(|d| d["kind"] == "helpful")
                .map(|d| d["concept"].clone())
                .collect::<Vec<_>>()
        );
        seen.insert(id.clone());
        let mut depth: BTreeMap<String, u64> = strings(&reg["dimensions"])
            .into_iter()
            .map(|s| (s, 0))
            .collect();
        for goal in &goals {
            for (dim, floor) in reg["objectives"][goal]
                .as_object()
                .context("invalid objective registry")?
            {
                *depth.get_mut(dim).context("unknown objective dimension")? =
                    depth[dim].max(floor.as_u64().context("invalid objective floor")?);
            }
        }
        for (kind, dim, floor) in [
            ("algorithmic", "theory", 3),
            ("programming", "code", 2),
            ("historical", "theory", 2),
            ("comparative", "theory", 2),
            ("procedural", "practice", 3),
            ("scientific", "practice", 2),
            ("research", "practice", 2),
        ] {
            if types.iter().any(|s| s == kind) {
                depth.insert(dim.into(), depth[dim].max(floor));
            }
        }
        let mut source_required = BTreeSet::new();
        for s in &sources {
            if s["required"] == true && strings(&s["concepts"]).contains(&id) {
                let choice = &p["SOURCE_MODULES"][s["kind"].as_str().unwrap()];
                let module = choice[0].as_str().unwrap_or("definitions");
                let dim = choice[1].as_str().unwrap_or("theory");
                let floor = choice[2].as_u64().unwrap_or(2);
                source_required.insert(module.to_string());
                depth.insert(dim.into(), depth[dim].max(floor));
            }
        }
        if let Some(overrides) = c["depth"].as_object() {
            for (dim, level) in overrides {
                let canonical = p["DIMENSION_ALIASES"][dim.to_lowercase()]
                    .as_str()
                    .context("unknown depth dimension")?;
                let level = level.as_u64().context("depth must be integer")?;
                ensure!(
                    level <= 5 && level >= depth[canonical],
                    "depth conflicts with required objective/type floor"
                );
                depth.insert(canonical.into(), level);
            }
        }
        c["depth"] = json!(depth);
        if c["implementation"].is_null() {
            c["implementation"] = json!({"required":depth["code"]>=2,"code_depth":match depth["code"]{0=>"none",1=>"pseudocode",2=>"minimal",3=>"complete",_=>"from-scratch"}});
        } else {
            ensure!(
                c["implementation"].is_object(),
                "implementation must be object"
            );
            ensure!(
                c["implementation"]["required"] != true || depth["code"] >= 2,
                "required implementation conflicts with code depth"
            );
        }
        if c["visualizations"].is_null() {
            c["visualizations"] = json!([]);
        }
        ensure!(
            c["visualizations"]
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_object)),
            "visualizations must be object list"
        );
        let mut modules = serde_json::Map::new();
        for (name, spec) in reg["modules"]
            .as_object()
            .context("missing module registry")?
        {
            let dim = spec["dimension"]
                .as_str()
                .context("module needs dimension")?;
            let required = source_required.contains(name)
                || strings(&spec["required_for"])
                    .iter()
                    .any(|s| goals.contains(s))
                || strings(&spec["types"]).iter().any(|s| types.contains(s));
            let status = if required {
                "required"
            } else if depth[dim] > 0
                && spec["supporting"] == true
                && supports(name, c, &depth, total)
            {
                "supporting"
            } else {
                "excluded"
            };
            modules.insert(name.clone(),json!({"status":status,"depth":if status=="excluded"{0}else{depth[dim]},"reason":if required{"source/objective/type requirement"}else if status=="supporting"{"selected educational need"}else{"no selected need"}}));
        }
        let custom = c["modules"].as_object().cloned().unwrap_or_default();
        for (name, choice) in &custom {
            let old = modules.get(name).context("unknown module")?;
            let status = choice["status"].as_str().context("module needs status")?;
            let level = choice["depth"].as_u64().context("module needs depth")?;
            let dim = reg["modules"][name]["dimension"].as_str().unwrap();
            ensure!(
                ["required", "supporting", "optional", "excluded"].contains(&status)
                    && level <= 5
                    && (status == "excluded") == (level == 0)
                    && level <= depth[dim],
                "invalid module status/depth"
            );
            ensure!(
                choice["reason"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty()),
                "module override requires reason"
            );
            ensure!(
                old["status"] != "required"
                    || (status == "required" && level >= old["depth"].as_u64().unwrap_or(0)),
                "cannot drop required module"
            );
            ensure!(
                !((name == "derivation" && level > 0 && level < 4)
                    || (name == "proof" && level > 0 && level < 5)),
                "derivation needs M4 and proof needs M5"
            );
            modules.insert(name.clone(), choice.clone());
        }
        let mut pending: Vec<String> = modules
            .iter()
            .filter(|(_, v)| {
                ["required", "supporting"].contains(&v["status"].as_str().unwrap_or(""))
            })
            .map(|(k, _)| k.clone())
            .collect();
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            for dep in strings(&reg["modules"][&name]["depends"]) {
                ensure!(
                    !custom.contains_key(&dep)
                        || !["excluded", "optional"]
                            .contains(&modules[&dep]["status"].as_str().unwrap_or("")),
                    "active module depends on deferred module"
                );
                if modules[&dep]["status"] != "required" {
                    let dim = reg["modules"][&dep]["dimension"].as_str().unwrap();
                    let level = depth[dim].max(2);
                    modules.insert(dep.clone(),json!({"status":"required","depth":level,"reason":format!("dependency of {name}")}));
                    depth.insert(dim.into(), level);
                }
                pending.push(dep);
            }
        }
        c["modules"] = Value::Object(modules);
        c["depth"] = json!(depth);
        c["composed_sequence"] = json!(compose_concept_sequence(c)?);
        c["sequence_candidates"] = json!(
            types
                .iter()
                .map(|kind| (kind.clone(), reg["subtemplates"][kind].clone()))
                .collect::<BTreeMap<_, _>>()
        );
        c["estimated_output_tokens"] = json!(
            180 + 90 * depth.values().sum::<u64>()
                + 120
                    * dependencies
                        .iter()
                        .filter(|d| ["explain", "foundation"]
                            .contains(&d["treatment"].as_str().unwrap_or("")))
                        .count() as u64
        );
    }
    let graph = build_concept_dependency_graph(&json!({"concepts":concepts}))?;
    let outline: Vec<_> = concepts
        .iter()
        .map(|c| json!({"concept":c["id"],"title":c["title"],"sequence":c["composed_sequence"]}))
        .collect();
    let mut result = raw.clone();
    result["version"] = json!(3);
    result["audience"] = json!(audience);
    result["source_policy"] = json!(source_policy);
    result["estimated_output_tokens"] = json!(
        concepts
            .iter()
            .map(|c| c["estimated_output_tokens"].as_u64().unwrap_or(0))
            .sum::<u64>()
    );
    result["concepts"] = json!(concepts);
    result["sources"] = json!(sources);
    result["dependency_graph"] = graph;
    result["composed_outline"] = json!({"concepts":outline});
    Ok(result)
}
pub fn build_concept_dependency_graph(plan: &Value) -> Result<Value> {
    let concepts = plan["concepts"].as_array().context("concepts needed")?;
    let mut edges = Vec::new();
    let mut gaps = Vec::new();
    let ids: BTreeSet<_> = concepts.iter().filter_map(|c| c["id"].as_str()).collect();
    for c in concepts {
        for dep in c["prerequisites"].as_array().into_iter().flatten() {
            let parent = dep["concept"]
                .as_str()
                .or_else(|| dep.as_str())
                .context("prerequisite needs ID")?;
            edges.push(
                json!({"from":parent,"to":c["id"],"kind":dep["kind"],"treatment":dep["treatment"]}),
            );
            if !ids.contains(parent) && dep["treatment"] != "known" {
                gaps.push(
                    json!({"concept":c["id"],"prerequisite":parent,"treatment":dep["treatment"]}),
                );
            }
        }
    }
    Ok(
        json!({"nodes":concepts.iter().map(|c|json!({"id":c["id"],"title":c["title"]})).collect::<Vec<_>>(),"edges":edges,"unresolved_foundations":gaps}),
    )
}
pub fn coverage_errors(plan: &Value, report: &Value, draft: &str) -> Vec<String> {
    let mut errors = Vec::new();
    for c in plan["concepts"].as_array().into_iter().flatten() {
        let id = c["id"].as_str().unwrap_or("");
        let title = c["title"].as_str().unwrap_or("");
        if title.is_empty() || !draft.contains(title) {
            errors.push(format!("missing concept: {id}"));
        }
        if let Some(modules) = c["modules"].as_object() {
            for (name, state) in modules {
                if state["status"] == "required"
                    && !report["concepts"][id]["modules"][name]["status"]
                        .as_str()
                        .is_some_and(|s| ["covered", "verified"].contains(&s))
                {
                    errors.push(format!("missing required module coverage: {id}:{name}"));
                }
            }
        }
    }
    errors
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn objectives_raise_floors_and_dependencies_are_preserved() {
        let p = json!({"concepts":[{"id":"a","title":"A","reason":"Foundation","types":["conceptual"],"objectives":["explain"]},{"id":"b","title":"B","reason":"Mechanism","types":["algorithmic"],"objectives":["trace"],"prerequisites":["a"]}]});
        let normalized = normalize_plan(&p).unwrap();
        assert_eq!(normalized["version"], 3);
        assert!(
            normalized["concepts"][1]["depth"]["theory"]
                .as_u64()
                .unwrap()
                >= 3
        );
        let mut bad = p;
        bad["concepts"][0]["prerequisites"] = json!(["b"]);
        assert!(normalize_plan(&bad).is_err());
    }
}
fn policy() -> Value {
    serde_json::from_str(r###"{"SOURCE_MODULES": {"equation": ["mathematics", "math", 2], "formula": ["mathematics", "math", 2], "derivation": ["derivation", "math", 4], "proof": ["proof", "math", 5], "figure": ["visualization", "visual", 2], "table": ["visualization", "visual", 2], "example": ["examples", "practice", 3], "exercise": ["assessment", "assessment", 3], "algorithm": ["algorithm", "theory", 3], "qualification": ["limitations", "theory", 2]}, "DIMENSION_ALIASES": {"math": "math", "mathematical": "math", "code": "code", "coding": "code", "visual": "visual", "theory": "theory", "conceptual": "theory", "theoretical": "theory", "algorithmic": "theory", "practice": "practice", "examples": "practice", "assessment": "assessment"}, "STAGE_CANONICAL": {"need": ["motivation_and_problem", "motivation"], "problem": ["motivation_and_problem", "motivation"], "purpose": ["motivation_and_problem", "motivation"], "goal": ["motivation_and_problem", "motivation"], "question": ["motivation_and_problem", "motivation"], "phenomenon": ["motivation_and_problem", "motivation"], "context": ["historical_context", "history"], "background": ["motivation_and_problem", "motivation"], "decision": ["motivation_and_problem", "comparison"], "intuition": ["intuition_and_model", "intuition"], "principles": ["intuition_and_model", "intuition"], "hypothesis": ["intuition_and_model", "intuition"], "definition": ["definitions_and_setup", "definitions"], "notation": ["definitions_and_setup", "definitions"], "assumptions": ["definitions_and_setup", "definitions"], "inputs_outputs": ["definitions_and_setup", "definitions"], "requirements": ["definitions_and_setup", "definitions"], "preparation": ["definitions_and_setup", "applications"], "state": ["definitions_and_setup", "algorithm"], "invariant": ["definitions_and_setup", "algorithm"], "criteria": ["definitions_and_setup", "comparison"], "components": ["mechanism_and_structure", "visualization"], "interfaces": ["mechanism_and_structure", "definitions"], "data_flow": ["mechanism_and_structure", "visualization"], "shapes": ["mechanism_and_structure", "visualization"], "interactions": ["mechanism_and_structure", "definitions"], "mechanism": ["mechanism_and_structure", "definitions"], "behavior": ["mechanism_and_structure", "definitions"], "chronology": ["chronology_and_developments", "history"], "developments": ["chronology_and_developments", "history"], "causes": ["chronology_and_developments", "history"], "consequences": ["chronology_and_developments", "history"], "relation": ["mathematical_formulation", "mathematics"], "justification": ["derivation_and_proof", "derivation"], "calculation": ["worked_application", "mathematics"], "algorithm": ["algorithmic_procedure", "algorithm"], "steps": ["algorithmic_procedure", "algorithm"], "intermediate_states": ["worked_application", "applications"], "environment": ["implementation_and_code", "coding"], "implementation": ["implementation_and_code", "coding"], "method": ["experiment_and_evidence", "experiment"], "experiment": ["experiment_and_evidence", "experiment"], "measurement": ["experiment_and_evidence", "experiment"], "evidence": ["experiment_and_evidence", "history"], "results": ["experiment_and_evidence", "experiment"], "examples": ["worked_application", "examples"], "trace": ["worked_application", "tracing"], "distinctions": ["comparison_and_distinctions", "definitions"], "shared_properties": ["comparison_and_distinctions", "comparison"], "differences": ["comparison_and_distinctions", "comparison"], "tradeoffs": ["comparison_and_distinctions", "comparison"], "conditional_choice": ["comparison_and_distinctions", "comparison"], "interpretations": ["interpretation_and_verification", "history"], "interpretation": ["interpretation_and_verification", "definitions"], "correctness": ["interpretation_and_verification", "algorithm"], "cost": ["interpretation_and_verification", "algorithm"], "tests": ["interpretation_and_verification", "coding"], "verification": ["interpretation_and_verification", "applications"], "debugging": ["limitations_and_boundaries", "coding"], "troubleshooting": ["limitations_and_boundaries", "applications"], "validity": ["limitations_and_boundaries", "mathematics"], "limits": ["limitations_and_boundaries", "limitations"], "limitations": ["limitations_and_boundaries", "limitations"], "edge_cases": ["limitations_and_boundaries", "limitations"], "bottlenecks": ["limitations_and_boundaries", "limitations"], "error": ["limitations_and_boundaries", "experiment"], "open_questions": ["revision_and_consolidation", "revision"]}, "STAGE_ORDER": ["prerequisite_preparation", "motivation_and_problem", "historical_context", "intuition_and_model", "definitions_and_setup", "mechanism_and_structure", "chronology_and_developments", "mathematical_formulation", "derivation_and_proof", "algorithmic_procedure", "implementation_and_code", "experiment_and_evidence", "worked_application", "comparison_and_distinctions", "interpretation_and_verification", "limitations_and_boundaries", "assessment_and_practice", "revision_and_consolidation"]}"###).expect("valid checked-in policy")
}
