//! ARRIE policy, source inspection attestations and claim-level evidence contracts.
use crate::knowledge::strings;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
fn policy() -> Value {
    serde_json::from_str(include_str!(
        "../../../references/workflows/research/research-policy-registry.json"
    ))
    .expect("checked-in research registry")
}
fn text<'a>(v: &'a Value, k: &str, d: &'a str) -> &'a str {
    v[k].as_str().unwrap_or(d)
}
fn required<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    let s = v[k].as_str().with_context(|| format!("missing {k}"))?;
    ensure!(!s.trim().is_empty(), "empty {k}");
    Ok(s)
}
pub fn resolve_research_mode(input: &Value) -> Result<Value> {
    let p = policy();
    let req = text(input, "request", "").to_lowercase();
    let source = text(input, "source_policy", "adaptive");
    let explicit = input["explicit_mode"].as_str().or(input["mode"].as_str());
    let code = if let Some(explicit) = explicit {
        p["modes"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(id, s)| id.as_str() == explicit || s["name"].as_str() == Some(explicit))
            .map(|(id, _)| id.clone())
            .context("unknown research mode")?
    } else if req.contains("no external")
        || req.contains("source-only")
        || req.contains("only from this")
        || req.contains("only from the source")
    {
        "R-S0".into()
    } else if source == "strict" {
        if req.contains("verify") || req.contains("fact-check") || req.contains("check against") {
            "R-S1".into()
        } else {
            "R-S0".into()
        }
    } else if source == "expanded" || source == "source_first" {
        "R-S2".into()
    } else if req.contains("literature review") || req.contains("systematic review") {
        "R-L".into()
    } else if req.contains("resources")
        && (req.contains("recommend") || req.contains("best") || req.contains("free"))
    {
        "R-R".into()
    } else if source == "comparative"
        || req.contains("compare")
        || req.contains("versus")
        || req.contains("tradeoff")
    {
        "R-C".into()
    } else if input["has_supplied_sources"] == true {
        "R-S2".into()
    } else {
        "R-I".into()
    };
    let mut result = p["modes"][&code].clone();
    result["mode"] = json!(code);
    Ok(result)
}
pub fn normalize_research_plan(raw: &Value) -> Result<Value> {
    let p = policy();
    let topic = required(raw, "topic")?;
    let mode = resolve_research_mode(&json!({"explicit_mode":text(raw,"mode","R-I")}))?;
    let mut seen = BTreeSet::new();
    let mut questions = Vec::new();
    let mut budget = 0;
    for (index, q) in raw["research_questions"]
        .as_array()
        .context("research_questions array required")?
        .iter()
        .enumerate()
    {
        let id = q["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("RQ-{:03}", index + 1));
        ensure!(seen.insert(id.clone()), "duplicate research question id");
        let concept = required(q, "concept_id")?;
        let question = required(q, "question")?;
        let kind = text(q, "evidence_type", "factual_verification");
        let spec = &p["evidence_types"][kind];
        ensure!(!spec.is_null(), "unknown evidence type");
        let depth = q
            .get("research_depth")
            .unwrap_or(&spec["default_depth"])
            .as_u64()
            .context("research_depth must be integer")?;
        ensure!(depth <= 5, "research depth must be 0..5");
        ensure!(
            depth == 0 || mode["allow_external_browsing"] == true,
            "research mode forbids external retrieval"
        );
        let priority = text(q, "priority", "medium");
        ensure!(
            ["high", "medium", "low"].contains(&priority),
            "invalid priority"
        );
        let status = if depth == 0 && text(q, "status", "pending") == "pending" {
            "skipped_r0"
        } else {
            text(q, "status", "pending")
        };
        ensure!(
            [
                "pending",
                "in_progress",
                "answered",
                "partially_answered",
                "unresolved",
                "skipped_r0"
            ]
            .contains(&status),
            "invalid research status"
        );
        let query = format!("{topic} {question}");
        let mut out = q.clone();
        out["id"] = json!(id);
        out["concept_id"] = json!(concept);
        out["question"] = json!(question);
        out["evidence_type"] = json!(kind);
        out["research_depth"] = json!(depth);
        out["research_depth_code"] = json!(format!("R{depth}"));
        out["preferred_source_types"] = q
            .get("preferred_source_types")
            .cloned()
            .unwrap_or_else(|| spec["source_preferences"].clone());
        ensure!(
            out["preferred_source_types"]
                .as_array()
                .is_some_and(|x| !x.is_empty()),
            "source preferences required"
        );
        out["priority"] = json!(priority);
        out["status"] = json!(status);
        out["completion_criteria"] = json!(text(
            q,
            "completion_criteria",
            "Inspect primary evidence, record exact locator, and disclose unresolved claims."
        ));
        out["queries"] = q
            .get("queries")
            .cloned()
            .unwrap_or(json!([{"query_type":"targeted","query":query}]));
        if depth == 0 {
            out["queries"] = json!([]);
        }
        budget += p["research_depths"][depth.to_string()]["soft_query_budget"]
            .as_u64()
            .unwrap_or(0);
        questions.push(out);
    }
    let mut result = raw.clone();
    for key in ["mode", "source_policy", "allow_external_browsing"] {
        result[key] = mode[key].clone();
    }
    result["mode_name"] = mode["name"].clone();
    result["research_plan_version"] = json!(1);
    result["research_questions"] = json!(questions);
    result["soft_query_budget"] = json!(budget);
    Ok(result)
}
fn infer_evidence_needs(c: &Value, topic: &str, request: &str, mode: &str) -> Vec<Value> {
    let title = text(c, "title", "");
    let context = format!("{topic} {title} {request}").to_lowercase();
    let types = strings(&c["types"]);
    let objectives = strings(&c["objectives"]);
    let has = |s: &str| types.iter().chain(objectives.iter()).any(|v| v == s);
    let mut needs = Vec::new();
    let mut add = |kind: &str, depth: u64, question: String| {
        let depth = if mode == "R-L" {
            5
        } else if mode == "R-C"
            && [
                "empirical_result",
                "software_architecture",
                "conceptual_expansion",
            ]
            .contains(&kind)
        {
            depth.max(4)
        } else {
            depth
        };
        needs.push(json!({"evidence_type":kind,"research_depth":depth,"priority":"high","question":question}));
    };
    if mode == "R-S0" {
        add(
            "no_external_research",
            0,
            format!(
                "Verify definitions, assumptions and claims for '{title}' within supplied source."
            ),
        );
        return needs;
    }
    let stable = [
        "polynomial",
        "power rule",
        "quadratic",
        "arithmetic",
        "chain rule",
        "product rule",
        "simple integral",
        "elementary algebra",
    ]
    .iter()
    .any(|k| context.contains(k));
    let current = [
        "version",
        "api",
        "recent",
        "latest",
        "benchmark",
        "paper",
        "theorem",
    ]
    .iter()
    .any(|k| context.contains(k));
    if stable
        && !current
        && !has("prove")
        && !has("programming")
        && !["R-L", "R-C", "R-R"].contains(&mode)
    {
        add(
            "no_external_research",
            0,
            format!(
                "Confirm internal mathematical consistency of '{title}' without unnecessary retrieval."
            ),
        );
        return needs;
    }
    if mode == "R-R" {
        add(
            "learning_resource",
            2,
            format!("Identify verified level-appropriate learning resources for '{title}'."),
        );
        return needs;
    }
    if mode == "R-L" {
        add(
            "literature_synthesis",
            5,
            format!(
                "Synthesize primary literature, competing findings and open questions for '{title}'."
            ),
        );
        return needs;
    }
    if has("prove")
        || c["depth"]["math"].as_u64().unwrap_or(0) >= 5
        || ["theorem", "convergence", "lemma", "proof"]
            .iter()
            .any(|k| context.contains(k))
    {
        add(
            "mathematical_proof",
            if has("prove") { 3 } else { 2 },
            format!(
                "Verify exact theorem statement, hypotheses, assumptions and proof conditions for '{title}'."
            ),
        );
    } else if has("derive")
        || has("calculate")
        || has("mathematical")
        || c["depth"]["math"].as_u64().unwrap_or(0) >= 2
    {
        add(
            "mathematical_definition",
            2,
            format!("Verify formal definition, notation and domain conditions for '{title}'."),
        );
    }
    if has("programming")
        || has("implement")
        || has("debug")
        || c["depth"]["code"].as_u64().unwrap_or(0) >= 2
    {
        let depth = if [
            "numpy",
            "pytorch",
            "torch",
            "tensorflow",
            "keras",
            "scikit-learn",
            "pandas",
            "api",
            "version",
            "deprecat",
        ]
        .iter()
        .any(|k| context.contains(k))
        {
            3
        } else {
            2
        };
        add(
            "programming_api",
            depth,
            format!(
                "Verify official API signatures, version constraints and runtime behavior for '{title}'."
            ),
        );
    }
    if has("architectural") {
        add(
            "software_architecture",
            2,
            format!(
                "Verify original architecture, component interfaces and data-flow contracts for '{title}'."
            ),
        );
    }
    if has("historical") {
        add(
            "historical_claim",
            2,
            format!("Verify chronology, primary records and scholarly attributions for '{title}'."),
        );
    }
    if has("scientific") {
        add(
            "scientific_measurement",
            2,
            format!(
                "Verify experimental conditions, units, error bounds and applicability regime for '{title}'."
            ),
        );
    }
    if has("research") || has("comparative") || mode == "R-C" {
        add(
            "empirical_result",
            if mode == "R-C" { 4 } else { 3 },
            format!("Verify methodology, datasets, metrics and competing findings for '{title}'."),
        );
    }
    if needs.is_empty() {
        let theory = c["depth"]["theory"].as_u64().unwrap_or(0);
        needs.push(json!({"evidence_type":if theory<=2{"factual_verification"}else{"conceptual_expansion"},"research_depth":if theory<=2{1}else{2},"priority":"medium","question":format!("Verify core definition, distinctions and boundary conditions for '{title}'.")}));
    }
    needs
}
pub fn plan_research(input: &Value) -> Result<Value> {
    let topic = required(input, "topic")?;
    let plan = input.get("instructional_plan").unwrap_or(input);
    let concepts = plan["concepts"]
        .as_array()
        .context("instructional concepts required")?;
    ensure!(!concepts.is_empty(), "empty concepts");
    let mode = resolve_research_mode(
        &json!({"request":input["request"],"source_policy":plan["source_policy"],"explicit_mode":input["explicit_mode"],"has_supplied_sources":plan["sources"].as_array().is_some_and(|s|!s.is_empty())}),
    )?;
    let questions = if let Some(q) = input["explicit_questions"].as_array() {
        q.clone()
    } else {
        let mut questions = Vec::new();
        for c in concepts {
            let id = c["id"]
                .as_str()
                .or(c["concept_id"].as_str())
                .context("concept id required")?;
            let title = required(c, "title")?;
            let needs = c["evidence_needs"].as_array().cloned().unwrap_or_else(|| {
                infer_evidence_needs(
                    c,
                    topic,
                    text(input, "request", ""),
                    mode["mode"].as_str().unwrap(),
                )
            });
            for need in needs {
                let mut q = need;
                q["concept_id"] = json!(id);
                q["concept_title"] = json!(title);
                if mode["allow_external_browsing"] == false {
                    q["research_depth"] = json!(0);
                }
                questions.push(q);
            }
        }
        questions
    };
    normalize_research_plan(
        &json!({"topic":topic,"mode":mode["mode"],"research_questions":questions,"offline":input["offline"].as_bool().unwrap_or(false)}),
    )
}
fn canonical_url(raw: &str) -> Result<String> {
    let mut url = url::Url::parse(raw)?;
    ensure!(
        ["https", "http"].contains(&url.scheme())
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none(),
        "invalid source URL"
    );
    url.set_fragment(None);
    ensure!(
        url.path() != "/" || url.query().is_some(),
        "bare root-domain citation forbidden"
    );
    let query: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_") && k != "fbclid")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.set_query(None);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Ok(url.to_string())
}
pub fn evaluate_source(input: &Value) -> Result<Value> {
    let p = policy();
    let source = input.get("source").unwrap_or(input);
    let kind = text(input, "evidence_type", "factual_verification");
    let spec = &p["evidence_types"][kind];
    ensure!(!spec.is_null(), "unknown evidence type");
    let stype = text(source, "source_type", "general_web");
    let role = p["source_roles"][stype]
        .as_str()
        .context("unknown source type")?;
    let mut reject = Vec::new();
    let mut warnings = Vec::new();
    if source["content_verified"] != true {
        reject.push("source content was not inspected");
    }
    if [
        "unavailable",
        "paywalled_uninspected",
        "metadata_only",
        "not_checked",
    ]
    .contains(&text(source, "access_status", "not_checked"))
    {
        reject.push("source access does not establish inspected content");
    }
    if let Some(url) = source["canonical_url"].as_str()
        && canonical_url(url).is_err()
    {
        reject.push("invalid or bare-root source locator");
    }
    if kind == "programming_api"
        && let Some(version) = input["required_version"].as_str()
        && source["version"].as_str() != Some(version)
    {
        reject.push("missing or mismatched API version");
    }
    if kind == "empirical_result" && source["has_methodology"] == false {
        reject.push("empirical claim lacks methodology");
    }
    if spec["primary_required"] == true && role == "tertiary" {
        reject.push("tertiary source cannot establish primary-required claim");
    }
    if source["is_preprint"] == true || stype == "preprint" {
        warnings.push("Preprint status requires disclosure");
    }
    let prefs = strings(&spec["source_preferences"]);
    let score = if let Some(index) = prefs.iter().position(|t| t == stype) {
        (1.0 - 0.1 * index as f64).max(0.55)
    } else {
        match role {
            "primary" => 0.7,
            "secondary" => 0.5,
            _ => 0.25,
        }
    };
    Ok(
        json!({"source_id":source["source_id"],"evidence_type":kind,"source_role":role,"accepted":reject.is_empty(),"authority_score":if reject.is_empty(){score}else{0.0},"warnings":warnings,"rejection_reasons":reject}),
    )
}
pub fn check_source_independence(sources: &[Value]) -> Value {
    let lookup:BTreeMap<_,_>=sources.iter().filter_map(|s|s["source_id"].as_str().map(|id|(id,s))).collect();
    let mut clusters: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut derivative = BTreeMap::new();
    for s in sources {
        let sid = text(s, "source_id", "");
        let mut current=s;let mut visited=BTreeSet::new();
        let origin=loop{let current_id=text(current,"source_id","");if !visited.insert(current_id.to_owned()){break "unresolved_origin_cycle".to_owned();}if let Some(parent)=current["origin_source_id"].as_str().or(current["cites_origin"].as_str()){if let Some(source)=lookup.get(parent){current=source;}else{break parent.trim().to_lowercase();}}else{break current["identifiers"]["doi"].as_str().or(current["canonical_url"].as_str()).unwrap_or(current_id).trim().to_lowercase();}};
        clusters.entry(origin).or_default().push(sid.into());
        if let Some(parent) = s["origin_source_id"]
            .as_str()
            .or(s["cites_origin"].as_str())
        {
            derivative.insert(sid, parent);
        }
    }
    let count=clusters.keys().filter(|s|s.as_str()!="unresolved_origin_cycle").count();
    json!({"total_sources":sources.len(),"independent_origin_count":count,"has_independent_corroboration":count>=2,"origin_clusters":clusters,"derivative_sources":derivative})
}
pub fn normalize_evidence_registry(input: &Value) -> Result<Value> {
    let raw = input.get("registry").unwrap_or(input);
    let p = policy();
    let mode = text(input, "mode", text(raw, "mode", "R-I"));
    let mode_spec = &p["modes"][mode];
    ensure!(!mode_spec.is_null(), "unknown research mode");
    let mut ids = BTreeSet::new();
    let mut urls = BTreeSet::new();
    let mut sources = Vec::new();
    for s in raw["sources"].as_array().context("sources required")? {
        let id = required(s, "source_id")?;
        required(s, "title")?;
        ensure!(ids.insert(id.to_owned()), "duplicate source id");
        let stype = text(s, "source_type", "general_web");
        ensure!(!p["source_roles"][stype].is_null(), "unknown source type");
        ensure!(
            mode_spec["allow_external_browsing"] == true || stype == "local_user_source",
            "strict mode forbids external source"
        );
        let access = text(s, "access_status", "not_checked");
        ensure!(
            [
                "full",
                "partial",
                "metadata_only",
                "unavailable",
                "not_checked",
                "paywalled_uninspected"
            ]
            .contains(&access),
            "invalid access status"
        );
        ensure!(
            s["content_verified"] != true || ["full", "partial"].contains(&access),
            "content_verified contradicts access"
        );
        let mut out = s.clone();
        if let Some(url) = s["canonical_url"].as_str() {
            let url = canonical_url(url)?;
            ensure!(urls.insert(url.clone()), "duplicate canonical URL");
            out["canonical_url"] = json!(url);
        }
        out["source_type"] = json!(stype);
        out["source_role"] = p["source_roles"][stype].clone();
        out["private"] = json!(
            s["private"]
                .as_bool()
                .unwrap_or(stype == "local_user_source")
        );
        sources.push(out);
    }
    let questions: BTreeSet<_> = input["research_plan"]["research_questions"]
        .as_array()
        .map(|qs| qs.iter().filter_map(|q| q["id"].as_str()).collect())
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut evidence = Vec::new();
    for e in raw["evidence"].as_array().context("evidence required")? {
        let id = required(e, "evidence_id")?;
        ensure!(seen.insert(id), "duplicate evidence id");
        let rq = required(e, "research_question_id")?;
        ensure!(
            questions.is_empty() || questions.contains(rq),
            "unknown question id"
        );
        required(e, "concept_id")?;
        required(e, "claim")?;
        let status = text(e, "support_status", "not_verified");
        ensure!(
            p["verification_states"]
                .as_object()
                .is_some_and(|s| s.contains_key(status))
                || p["verification_states"]
                    .as_array()
                    .is_some_and(|a| a.contains(&json!(status))),
            "invalid evidence status"
        );
        let linked = strings(&e["source_ids"]);
        ensure!(
            linked.iter().all(|id| ids.contains(id)),
            "unknown evidence source"
        );
        let supported = [
            "supported_directly",
            "supported_with_qualifications",
            "supported_indirectly",
            "corroborated_independently",
        ]
        .contains(&status);
        let inspected: Vec<_> = sources
            .iter()
            .filter(|s| {
                linked.contains(&s["source_id"].as_str().unwrap_or("").to_owned())
                    && s["content_verified"] == true
            })
            .cloned()
            .collect();
        if supported {
            ensure!(
                !inspected.is_empty(),
                "supported evidence requires inspected source"
            );
            ensure!(
                !strings(&e["source_locations"]).is_empty()
                    || e["source_locations"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty()),
                "supported evidence requires exact source locator"
            );
        }
        if status == "corroborated_independently" {
            ensure!(
                check_source_independence(&inspected)["has_independent_corroboration"] == true,
                "independent corroboration requires two origins"
            );
        }
        if status == "computationally_verified" {
            ensure!(
                !text(e, "methodology", "").is_empty() && !e["execution_receipt"].is_null(),
                "computational verification requires methodology and execution receipt"
            );
        }
        let mut out = e.clone();
        out["support_status"] = json!(status);
        evidence.push(out);
    }
    Ok(
        json!({"mode":mode,"sources":sources,"evidence":evidence,"independence":check_source_independence(&sources)}),
    )
}
pub fn analyze_research_coverage(input: &Value) -> Result<Value> {
    let plan = input["research_plan"]
        .as_object()
        .context("research_plan required")?;
    let registry = &input["evidence_registry"];
    let evidence = registry["evidence"]
        .as_array()
        .context("evidence registry required")?;
    let mut missing = Vec::new();
    let mut coverage = Vec::new();
    let mut question_coverage = BTreeMap::new();
    let mut concept_states = BTreeMap::<String, Vec<String>>::new();
    let mut gaps = Vec::new();
    for q in plan["research_questions"]
        .as_array()
        .context("questions required")?
    {
        let supported: Vec<_> = evidence
            .iter()
            .filter(|e| {
                e["research_question_id"] == q["id"]
                    && [
                        "supported_directly",
                        "supported_with_qualifications",
                        "supported_indirectly",
                        "corroborated_independently",
                        "computationally_verified",
                    ]
                    .contains(&text(e, "support_status", "not_verified"))
            })
            .collect();
        let records: Vec<_> = evidence
            .iter()
            .filter(|e| e["research_question_id"] == q["id"])
            .collect();
        let state = if q["research_depth"] == 0 {
            if records.is_empty() {
                "not_applicable"
            } else {
                "complete"
            }
        } else if records.is_empty() {
            "missing"
        } else if records
            .iter()
            .any(|e| e["support_status"] == "contradicted")
        {
            "partial"
        } else if records.iter().all(|e| {
            [
                "supported_directly",
                "corroborated_independently",
                "computationally_verified",
            ]
            .contains(&text(e, "support_status", "not_verified"))
        }) {
            "complete"
        } else if supported.len() == records.len() {
            "sufficient_with_qualifications"
        } else if supported.is_empty() {
            "missing"
        } else {
            "partial"
        };
        if ["missing", "partial"].contains(&state) {
            missing.push(q["id"].clone());
            gaps.push(json!({"research_question_id":q["id"],"concept_id":q["concept_id"],"priority":q["priority"],"reason":if records.iter().any(|e|e["support_status"]=="contradicted"){"Contradictory evidence requires qualification and methodology comparison"}else{"Required claims remain unresolved"}}));
        }
        question_coverage.insert(q["id"].as_str().unwrap_or("").to_owned(),json!({"concept_id":q["concept_id"],"priority":q["priority"],"coverage":state,"evidence_count":records.len()}));
        concept_states
            .entry(q["concept_id"].as_str().unwrap_or("").to_owned())
            .or_default()
            .push(state.into());
        coverage.push(json!({"id":q["id"],"status":state,"evidence_ids":supported.iter().map(|e|&e["evidence_id"]).collect::<Vec<_>>()}));
    }
    let concept_coverage: BTreeMap<_, _> = concept_states
        .into_iter()
        .map(|(id, states)| {
            let state = if states.iter().all(|s| s == "not_applicable") {
                "not_applicable"
            } else if states.iter().any(|s| s == "missing") {
                if states
                    .iter()
                    .all(|s| s == "missing" || s == "not_applicable")
                {
                    "missing"
                } else {
                    "partial"
                }
            } else if states.iter().any(|s| s == "partial") {
                "partial"
            } else if states.iter().any(|s| s == "sufficient_with_qualifications") {
                "sufficient_with_qualifications"
            } else {
                "complete"
            };
            (id, state)
        })
        .collect();
    let uncovered: Vec<_> = input["source_inventory"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| {
            e["required"] == true
                && (e["access"] != "full"
                    || !evidence.iter().any(|record| {
                        strings(&record["source_locations"]).iter().any(|loc| {
                            loc.to_lowercase()
                                .contains(&text(e, "id", "").to_lowercase())
                        })
                    }))
        })
        .map(|e| e["id"].clone())
        .collect();
    Ok(
        json!({"complete":missing.is_empty()&&uncovered.is_empty(),"coverage":coverage,"question_coverage":question_coverage,"concept_coverage":concept_coverage,"gaps":gaps,"uncovered_inventory_ids":uncovered,"all_high_priority_covered":!gaps.iter().any(|g|g["priority"]=="high")&&uncovered.is_empty(),"unresolved_questions":missing,"semantic_review_required":true}),
    )
}
pub fn research_package(input: &Value) -> Result<Value> {
    let registry = normalize_evidence_registry(
        &json!({"registry":input["evidence_registry"],"mode":input["research_plan"]["mode"],"research_plan":input["research_plan"]}),
    )?;
    let coverage = analyze_research_coverage(
        &json!({"research_plan":input["research_plan"],"evidence_registry":registry}),
    )?;
    Ok(
        json!({"research_plan":input["research_plan"],"evidence_registry":registry,"coverage":coverage,"complete":coverage["complete"]}),
    )
}
pub fn curate_learning_resources(input: &Value) -> Result<Value> {
    let p = policy();
    let audience = text(input, "audience", "beginner");
    ensure!(
        ["beginner", "intermediate", "advanced"].contains(&audience),
        "invalid learner level"
    );
    let candidates = input["candidates"]
        .as_array()
        .context("candidates required")?;
    let stages = strings(&p["progression_stages"]);
    let objectives: BTreeSet<_> = strings(&input["objectives"]).into_iter().collect();
    let mut selected = Vec::new();
    let mut excluded = Vec::new();
    let mut paywalled = Vec::new();
    for c in candidates {
        let title = text(c, "title", "");
        let cost = text(c, "cost", "unknown");
        let mut reason = None;
        if title.trim().is_empty()
            || [
                "textbooks",
                "online courses",
                "youtube",
                "read more",
                "search online",
            ]
            .contains(&title.to_lowercase().as_str())
        {
            reason = Some("vague or non-specific recommendation");
        } else if c["verified"] != true {
            reason = Some("resource existence or access not verified");
        } else if let Some(url) = c["canonical_url"].as_str()
            && canonical_url(url).is_err()
        {
            reason = Some("invalid or bare-root resource URL");
        }
        if reason.is_none() && input["free_only"] == true && cost != "free" {
            if ["paid", "freemium", "institutional"].contains(&cost) {
                paywalled.push(c.clone());
                continue;
            }
            reason = Some("cost not verified as free");
        }
        let mut levels = strings(&c["learner_levels"]);
        if levels.is_empty() {
            levels.push(text(c, "learner_level", "beginner").into());
        }
        if reason.is_none() && !levels.iter().any(|l| l == audience) {
            reason = Some("learner level mismatch");
        }
        if reason.is_none() && text(c, "why_useful", "").trim().is_empty() {
            reason = Some("missing pedagogical justification");
        }
        if let Some(reason) = reason {
            excluded.push(json!({"title":title,"reason":reason}));
            continue;
        }
        let stage = text(
            c,
            "progression_stage",
            if audience == "advanced" {
                "advanced_theory"
            } else {
                "conceptual_foundation"
            },
        );
        let rank = stages.iter().position(|s| s == stage).unwrap_or(0);
        let overlap = strings(&c["objectives"])
            .iter()
            .filter(|o| objectives.contains(*o))
            .count();
        let score = (
            rank,
            if c["priority"] == "essential" { 0 } else { 1 },
            usize::MAX - overlap,
            title.to_lowercase(),
        );
        let mut entry = c.clone();
        entry["learner_level"] = json!(audience);
        entry["cost"] = json!(cost);
        entry["progression_stage"] = json!(stages.get(rank).map(String::as_str).unwrap_or(stage));
        if let Some(url) = c["canonical_url"].as_str() {
            entry["canonical_url"] = json!(canonical_url(url)?);
        }
        selected.push((score, entry));
    }
    selected.sort_by(|a, b| a.0.cmp(&b.0));
    let limit = if input["extensive"] == true {
        selected.len()
    } else {
        input["max_items"].as_u64().unwrap_or(5).clamp(1, 100) as usize
    };
    Ok(
        json!({"audience":audience,"free_only":input["free_only"].as_bool().unwrap_or(false),"recommended":selected.into_iter().take(limit).map(|(_,e)|e).collect::<Vec<_>>(),"paywalled_alternatives":paywalled,"excluded":excluded}),
    )
}
/// Day-level ISO-8601 freshness evaluation. Full timestamps retain their source date.
pub fn source_freshness(input: &Value) -> Result<Value> {
    let p = policy();
    let kind = text(input, "evidence_type", "factual_verification");
    let window = p["evidence_types"][kind]["freshness_days"]
        .as_i64()
        .context("unknown evidence type")?;
    let date = input["last_updated"]
        .as_str()
        .or(input["publication_date"].as_str())
        .context("source date required")?;
    let d: Vec<_> = date
        .get(..10)
        .context("date must be YYYY-MM-DD")?
        .split('-')
        .map(str::parse::<i64>)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(d.len() == 3 && (1..=12).contains(&d[1]), "invalid date");
    let leap = d[0] % 4 == 0 && (d[0] % 100 != 0 || d[0] % 400 == 0);
    let monthdays = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    ensure!(
        d[2] >= 1 && d[2] <= monthdays[(d[1] - 1) as usize],
        "invalid day"
    );
    let year = d[0] - i64::from(d[1] <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = d[1] + if d[1] > 2 { -3 } else { 9 };
    let doy = (153 * month + 2) / 5 + d[2] - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    let now = input["now_unix_seconds"]
        .as_i64()
        .unwrap_or(crate::cache::now()? as i64)
        / 86400;
    ensure!(days <= now, "source date is in the future");
    Ok(
        json!({"fresh":now-days<=window,"age_days":now-days,"freshness_window_days":window,"evidence_type":kind,"precision":"UTC calendar day"}),
    )
}
pub fn should_stop_research(input: &Value) -> Result<Value> {
    let coverage = analyze_research_coverage(input)?;
    let mode = text(&input["research_plan"], "mode", "R-I");
    let budget = input["research_plan"]["soft_query_budget"]
        .as_u64()
        .unwrap_or(0);
    let used = input["queries_used"].as_u64().unwrap_or(0);
    let complete = coverage["complete"] == true;
    let offline = input["offline"] == true;
    let exhausted = input["budget_exhausted"] == true
        || (budget > 0 && used >= budget && input["marginal_gain_low"] == true);
    let stop = mode == "R-S0" || offline || complete || exhausted;
    Ok(
        json!({"stop":stop,"reason":if mode=="R-S0"{"strict source-only mode prohibits retrieval"}else if offline{"offline; disclose remaining gaps"}else if complete{"research questions sufficiently covered"}else if exhausted{"budget reached with unresolved gaps disclosed"}else{"required research gaps remain"},"coverage":coverage}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_math_theorem_and_api_keep_distinct_research_obligations() {
        let stable=plan_research(&json!({"topic":"Derivative of a polynomial","concepts":[{"id":"p","title":"Polynomial derivative","types":["mathematical"],"objectives":["calculate"]}]})).unwrap();
        assert_eq!(stable["research_questions"][0]["research_depth"], 0);
        assert_eq!(stable["research_questions"][0]["queries"], json!([]));
        assert_eq!(stable["soft_query_budget"], 0);
        let proof=plan_research(&json!({"topic":"Perceptron convergence theorem","concepts":[{"id":"t","title":"Convergence theorem","types":["mathematical"],"objectives":["prove"],"depth":{"math":5}}]})).unwrap();
        assert_eq!(
            proof["research_questions"][0]["evidence_type"],
            "mathematical_proof"
        );
        assert_eq!(proof["research_questions"][0]["research_depth"], 3);
        let api=plan_research(&json!({"topic":"PyTorch Conv2d API","concepts":[{"id":"a","title":"Conv2d API","types":["programming"],"objectives":["implement"]}]})).unwrap();
        assert_eq!(
            api["research_questions"][0]["evidence_type"],
            "programming_api"
        );
        assert_eq!(api["research_questions"][0]["research_depth"], 3);
    }
    #[test]
    fn contradictory_evidence_blocks_coverage_even_when_one_source_supports() {
        let report=analyze_research_coverage(&json!({"research_plan":{"research_questions":[{"id":"q","concept_id":"c","research_depth":2,"priority":"high"}]},"evidence_registry":{"evidence":[{"evidence_id":"a","research_question_id":"q","support_status":"supported_directly"},{"evidence_id":"b","research_question_id":"q","support_status":"contradicted"}]}})).unwrap();
        assert_eq!(report["complete"], false);
        assert_eq!(report["all_high_priority_covered"], false);
        assert_eq!(report["question_coverage"]["q"]["coverage"], "partial");
    }
    #[test]
    fn resources_require_verified_free_access_and_usefulness() {
        let result=curate_learning_resources(&json!({"free_only":true,"candidates":[{"title":"Unverified course","verified":false,"cost":"free"},{"title":"Paid course","verified":true,"cost":"paid","canonical_url":"https://example.org/course","why_useful":"Worked examples"},{"title":"Open book","verified":true,"cost":"free","canonical_url":"https://example.org/book","why_useful":"Worked examples","learner_levels":["beginner"]}]})).unwrap();
        assert_eq!(result["recommended"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["paywalled_alternatives"].as_array().unwrap().len(),
            1
        );
        assert_eq!(result["excluded"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn freshness_rejects_future_and_invalid_dates() {
        assert!(source_freshness(&json!({"evidence_type":"programming_api","last_updated":"2026-02-30","now_unix_seconds":1795300000})).is_err());
        assert!(source_freshness(&json!({"evidence_type":"programming_api","last_updated":"2040-01-01","now_unix_seconds":1795300000})).is_err());
        assert_eq!(source_freshness(&json!({"evidence_type":"programming_api","last_updated":"2020-01-01","now_unix_seconds":1795300000})).unwrap()["fresh"],false);
    }
    #[test]
    fn strict_modes_and_depth_gate() {
        assert_eq!(
            resolve_research_mode(&json!({"request":"no external sources"})).unwrap()["mode"],
            "R-S0"
        );
        assert!(normalize_research_plan(&json!({"topic":"Physics","mode":"R-S0","research_questions":[{"concept_id":"c","question":"What is energy?","research_depth":2}]})).is_err());
    }
    #[test]
    fn rejects_fake_source_and_bare_url() {
        assert!(normalize_evidence_registry(&json!({"sources":[{"source_id":"s","title":"Paper","canonical_url":"https://example.org/"}],"evidence":[]})).is_err());
        assert!(normalize_evidence_registry(&json!({"sources":[],"evidence":[{"evidence_id":"e","concept_id":"c","research_question_id":"q","claim":"a","source_ids":["fake"],"support_status":"supported_directly"}]})).is_err());
    }
    #[test]
    fn derivative_sources_do_not_count_independently() {
        assert_eq!(
            check_source_independence(&[
                json!({"source_id":"a","origin_source_id":"original"}),
                json!({"source_id":"b","origin_source_id":"original"})
            ])["independent_origin_count"],
            1
        );
    }
}
