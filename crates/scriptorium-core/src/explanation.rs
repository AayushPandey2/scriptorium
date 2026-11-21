//! Native contracts ported from the maintained instructional policy.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn resolve_eis_activation(
    master: &str,
    concept_type: &str,
    complexity: &str,
    source_mode: &str,
    audience: &str,
) -> Result<Value> {
    let p = policy();
    ensure!(
        p["MASTER_EIS_ACTIVATION"].get(master).is_some(),
        "unknown master"
    );
    let mut activation = p["MASTER_EIS_ACTIVATION"][master].clone();
    if activation["eis_05_state"] == "contextual" {
        activation["eis_05_state"] =
            json!(if ["algorithm", "computational"].contains(&concept_type) {
                "deep"
            } else if ["mathematical", "scientific"].contains(&concept_type) {
                "standard"
            } else {
                "disabled"
            });
    }
    if complexity == "trivial" || concept_type == "definition" {
        for (key, value) in [
            ("eis_01_spine", "lightweight"),
            ("eis_02_questions", "lightweight"),
            ("eis_03_motivation", "disabled"),
            ("eis_05_state", "disabled"),
            ("eis_06_retrieval", "lightweight"),
        ] {
            activation[key] = json!(value);
        }
    }
    if ["historical", "conceptual"].contains(&concept_type)
        && activation["eis_03_motivation"] == "deep"
    {
        activation["eis_03_motivation"] = json!("standard");
    }
    if source_mode == "strict" && master == "M2" {
        activation["eis_01_spine"] = json!("standard");
    }
    if audience == "advanced" && master == "M3" {
        activation["eis_02_questions"] = json!("disabled");
    }
    Ok(activation)
}
pub fn build_explanation_blueprint(input: &Value) -> Result<Value> {
    let topic = input["topic"].as_str().context("topic required")?;
    ensure!(!topic.trim().is_empty(), "empty topic");
    let master = input["master"].as_str().unwrap_or("M1");
    let audience = input["audience"].as_str().unwrap_or("intermediate");
    let source_mode = input["source_mode"].as_str().unwrap_or("adaptive");
    ensure!(
        ["strict", "source_first", "adaptive"].contains(&source_mode),
        "unknown source mode"
    );
    ensure!(
        [
            "beginner",
            "intermediate",
            "advanced",
            "cross_domain_expert"
        ]
        .contains(&audience),
        "unknown audience"
    );
    let concept_type = input["concept_type"].as_str().unwrap_or("conceptual");
    let family = match concept_type {
        "algorithm" | "computational" => "computational",
        "definition" => "conceptual",
        kind => kind,
    };
    let p = policy();
    ensure!(
        p["DOMAIN_SPINE_TEMPLATES"].get(family).is_some(),
        "unknown concept type"
    );
    let activation = resolve_eis_activation(
        master,
        concept_type,
        input["complexity"].as_str().unwrap_or("standard"),
        source_mode,
        audience,
    )?;
    let mut spine = Vec::new();
    for stage in p["DOMAIN_SPINE_TEMPLATES"][family].as_array().unwrap() {
        let name = stage.as_str().unwrap();
        let authored = input["spine"][name].as_str();
        spine.push(json!({"stage":name,"authored_content":authored,"status":if authored.is_some_and(|s|!s.trim().is_empty()){"provided"}else{"requires_authoring"},"verification":"review exact authored content and its evidence"}));
    }
    let central = input["central_question"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!("How does {topic} work, under what assumptions, and when should it be used?")
        });
    let mut questions = Vec::new();
    if activation["eis_02_questions"] != "disabled" {
        for q in input["anticipated_questions"]
            .as_array()
            .into_iter()
            .flatten()
        {
            ensure!(
                q["question"].as_str().is_some_and(|s| !s.trim().is_empty())
                    && q["purpose"].as_str().is_some_and(|s| !s.trim().is_empty()),
                "anticipated question needs purpose"
            );
            questions.push(q.clone());
        }
    }
    let mut variables = Vec::new();
    for v in input["variables"].as_array().into_iter().flatten() {
        for key in ["symbol", "meaning", "role", "domain", "units"] {
            ensure!(
                v[key].as_str().is_some_and(|s| !s.trim().is_empty()),
                "variable missing {key}"
            );
        }
        variables.push(v.clone());
    }
    let mut boundaries = Vec::new();
    for boundary in input["boundaries"].as_array().into_iter().flatten() {
        ensure!(
            boundary["condition"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
                && boundary["consequence"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()),
            "boundary needs condition and consequence"
        );
        boundaries.push(boundary.clone());
    }
    let mut cues = Vec::new();
    for cue in input["retrieval_cues"].as_array().into_iter().flatten() {
        ensure!(
            cue["cue"].as_str().is_some_and(|s| !s.is_empty())
                && cue["reconstruction"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()),
            "retrieval cue needs reconstruction"
        );
        cues.push(cue.clone());
    }
    Ok(
        json!({"schema_version":"1.0","topic":topic,"master":master,"audience":audience,"source_mode":source_mode,"concept_type":concept_type,"domain_family":family,"activation_levels":activation,"central_question":central,"spine":spine,"anticipated_questions":questions,"variable_roles":variables,"boundaries":boundaries,"retrieval_cues":cues,"source_order_preserved":input["preserve_source_order"].as_bool().unwrap_or(source_mode=="strict"),"external_facts_permitted":source_mode!="strict","review_required":true}),
    )
}
pub fn validate_explanation_structure(blueprint: &Value, body: &str) -> Vec<String> {
    let mut errors = Vec::new();
    if body.trim().is_empty() {
        errors.push("missing authored explanation".into());
    }
    let mut last = None;
    for stage in blueprint["spine"].as_array().into_iter().flatten() {
        let text = stage["authored_content"].as_str().unwrap_or("");
        if text.trim().is_empty() {
            errors.push(format!(
                "unfulfilled explanatory spine stage: {}",
                stage["stage"].as_str().unwrap_or("stage")
            ));
            continue;
        }
        match body.find(text) {
            Some(at) => {
                if last.is_some_and(|prev| at < prev) {
                    errors.push("explanatory spine violates causal/logical order".into());
                }
                last = Some(at);
            }
            None => errors.push("spine content missing from authored explanation".into()),
        }
    }
    for variable in blueprint["variable_roles"].as_array().into_iter().flatten() {
        let symbol = variable["symbol"].as_str().unwrap_or("");
        let meaning = variable["meaning"].as_str().unwrap_or("");
        if !body.contains(symbol) || !body.contains(meaning) {
            errors.push("variable role/meaning missing from explanation".into());
        }
    }
    errors
}
pub fn compose_explanation_note(
    blueprint: &Value,
    authored_body: &str,
    properties: &Value,
) -> Result<String> {
    let errors = validate_explanation_structure(blueprint, authored_body);
    ensure!(errors.is_empty(), "{}", errors.join("; "));
    ensure!(properties.is_object(), "properties must be mapping");
    let text = format!(
        "---\n{}---\n{}\n",
        serde_yaml_ng::to_string(properties)?,
        authored_body.trim()
    );
    crate::validation::validate_properties(&text)?;
    Ok(text)
}
pub fn verify_numerical_trace(trace: &Value) -> Result<Value> {
    let steps = trace["steps"].as_array().context("trace needs steps")?;
    ensure!(!steps.is_empty(), "trace cannot be empty");
    let mut outputs = BTreeMap::new();
    for step in steps {
        let id = step["id"].as_str().context("trace step needs ID")?;
        ensure!(!outputs.contains_key(id), "trace step IDs must be unique");
        let expression = step["expression"]
            .as_str()
            .context("trace step needs expression")?;
        let mut variables = step["variables"].as_object().cloned().unwrap_or_default();
        variables.extend(
            outputs
                .iter()
                .map(|(k, v): (&String, &f64)| (k.clone(), json!(v))),
        );
        let actual = crate::pedagogy::arithmetic(expression, &Value::Object(variables))?;
        if let Some(expected) = step["expected"].as_f64() {
            ensure!(
                (actual - expected).abs() <= 1e-12 + 1e-9 * actual.abs().max(expected.abs()),
                "numerical trace disagrees at {id}"
            );
        }
        outputs.insert(id.to_string(), actual);
    }
    Ok(
        json!({"verified":true,"values":outputs,"scope":"recomputed supplied arithmetic trace; not a universal proof"}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_keeps_master_and_source_boundaries() {
        let a =
            resolve_eis_activation("M5", "algorithm", "standard", "strict", "beginner").unwrap();
        assert_eq!(a["eis_05_state"], "deep");
        assert_eq!(
            resolve_eis_activation("M1", "definition", "trivial", "adaptive", "intermediate")
                .unwrap()["eis_03_motivation"],
            "disabled"
        );
    }
    #[test]
    fn planned_spine_never_pretends_to_be_authored() {
        let b = build_explanation_blueprint(&json!({"topic":"A mechanism","source_mode":"strict"}))
            .unwrap();
        assert_eq!(b["external_facts_permitted"], false);
        assert!(!validate_explanation_structure(&b, "A generic sentence.").is_empty());
    }
}
fn policy() -> Value {
    serde_json::from_str(r###"{"MASTER_EIS_ACTIVATION": {"M1": {"eis_01_spine": "deep", "eis_02_questions": "deep", "eis_03_motivation": "deep", "eis_04_boundaries": "deep", "eis_05_state": "contextual", "eis_06_retrieval": "standard", "eis_07_reader_review": "deep"}, "M2": {"eis_01_spine": "standard", "eis_02_questions": "standard", "eis_03_motivation": "standard", "eis_04_boundaries": "standard", "eis_05_state": "contextual", "eis_06_retrieval": "standard", "eis_07_reader_review": "standard"}, "M3": {"eis_01_spine": "lightweight", "eis_02_questions": "lightweight", "eis_03_motivation": "disabled", "eis_04_boundaries": "standard", "eis_05_state": "lightweight", "eis_06_retrieval": "lightweight", "eis_07_reader_review": "standard"}, "M4": {"eis_01_spine": "standard", "eis_02_questions": "lightweight", "eis_03_motivation": "deep", "eis_04_boundaries": "standard", "eis_05_state": "standard", "eis_06_retrieval": "standard", "eis_07_reader_review": "standard"}, "M5": {"eis_01_spine": "standard", "eis_02_questions": "standard", "eis_03_motivation": "deep", "eis_04_boundaries": "deep", "eis_05_state": "deep", "eis_06_retrieval": "standard", "eis_07_reader_review": "standard"}, "M6": {"eis_01_spine": "lightweight", "eis_02_questions": "standard", "eis_03_motivation": "disabled", "eis_04_boundaries": "standard", "eis_05_state": "disabled", "eis_06_retrieval": "lightweight", "eis_07_reader_review": "standard"}, "M7": {"eis_01_spine": "standard", "eis_02_questions": "lightweight", "eis_03_motivation": "standard", "eis_04_boundaries": "standard", "eis_05_state": "contextual", "eis_06_retrieval": "deep", "eis_07_reader_review": "standard"}}, "DOMAIN_SPINE_TEMPLATES": {"mathematical": ["problem", "relevant_property", "transformation", "result", "conditions"], "computational": ["task", "constraint", "algorithmic_insight", "procedure", "output"], "scientific": ["phenomenon", "model_assumptions", "governing_principle", "prediction", "limitations"], "historical": ["historical_question", "context", "developments", "competing_interpretations", "consequences"], "conceptual": ["question", "definition", "distinction", "implication", "application"]}}"###).expect("valid checked-in policy")
}
