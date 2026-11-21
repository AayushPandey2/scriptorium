//! Native adaptive chapter composition with explicit evidence and review gates.
use crate::{assessment, cache::hash, pedagogy};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    v[key]
        .as_array()
        .with_context(|| format!("{key} must be an array"))
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    let s = v[key]
        .as_str()
        .with_context(|| format!("missing text {key}"))?;
    ensure!(!s.trim().is_empty(), "empty {key}");
    Ok(s)
}
fn strings(v: &Value) -> Result<Vec<String>> {
    v.as_array()
        .context("expected string list")?
        .iter()
        .map(|s| Ok(s.as_str().context("expected string")?.into()))
        .collect()
}
fn list(v: &Value, key: &str) -> Vec<Value> {
    v[key].as_array().cloned().unwrap_or_default()
}
fn slug(value: &str) -> String {
    let mut out = String::new();
    for c in value.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c)
        } else if !out.ends_with('-') {
            out.push('-')
        }
    }
    let s = out.trim_matches('-');
    if s.is_empty() {
        "note".into()
    } else {
        s.into()
    }
}
pub fn safe_title(value: &str) -> Result<String> {
    let value = value.trim().trim_end_matches(['.', ' ']);
    ensure!(
        !value.is_empty()
            && !value
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)),
        "unsafe title"
    );
    let stem = value.split('.').next().unwrap().to_ascii_uppercase();
    ensure!(
        ![
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"
        ]
        .contains(&stem.as_str()),
        "reserved filename"
    );
    Ok(value.into())
}
fn chapter_title(value: &str) -> Result<String> {
    let clean = value
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let clean = clean.chars().take(90).collect::<String>();
    safe_title(&clean).or_else(|_| safe_title(&format!("Concept {clean}")))
}
type ConceptParents = BTreeMap<String, Vec<String>>;
fn inventory(concepts: &[Value], known: &[String]) -> Result<(ConceptParents, Vec<Value>)> {
    ensure!(
        concepts.len() >= 5,
        "at least five substantive concepts needed; narrow the output profile instead of padding"
    );
    let mut ids = BTreeSet::new();
    for c in concepts {
        let id = text(c, "id")?;
        ensure!(ids.insert(id.to_string()), "duplicate concept identity");
        for field in ["title", "section"] {
            let s = c[field]
                .as_str()
                .or_else(|| c["title"].as_str())
                .context("concept needs title/section")?;
            ensure!(
                !s.trim().is_empty() && !s.contains(['\n', '\r']),
                "concept title/section must be single line"
            );
        }
        ensure!(
            !array(c, "objectives")?.is_empty(),
            "concept needs objectives"
        );
        for goal in array(c, "objectives")? {
            ensure!(
                goal.as_str().is_some_and(|s| !s.trim().is_empty()),
                "invalid objective"
            );
        }
    }
    let mut parents = BTreeMap::new();
    let mut external = Vec::new();
    for c in concepts {
        let id = text(c, "id")?;
        let mut owned = Vec::new();
        for dep in list(c, "prerequisites") {
            let name = dep
                .as_str()
                .or_else(|| dep["id"].as_str())
                .context("prerequisite needs identity")?;
            let classification = dep["classification"]
                .as_str()
                .unwrap_or(if ids.contains(name) {
                    "covered"
                } else {
                    "assumed"
                });
            ensure!(
                [
                    "essential",
                    "supporting",
                    "optional",
                    "covered",
                    "assumed",
                    "missing"
                ]
                .contains(&classification),
                "invalid prerequisite classification"
            );
            if ids.contains(name) {
                if !owned.iter().any(|s| s == name) {
                    owned.push(name.to_string());
                }
            } else {
                ensure!(
                    known.iter().any(|s| s == name)
                        || ["optional", "missing"].contains(&classification),
                    "unknown prerequisite {name}"
                );
                external.push(json!({"concept":id,"id":name,"classification":classification,"known":known.iter().any(|s|s==name)}));
            }
        }
        parents.insert(id.into(), owned);
    }
    Ok((parents, external))
}
pub fn plan_chapters(concepts: &[Value], known: &[String]) -> Result<Value> {
    let (parents, external) = inventory(concepts, known)?;
    let records: BTreeMap<_, _> = concepts
        .iter()
        .map(|c| (c["id"].as_str().unwrap().to_string(), c))
        .collect();
    let mut ordered: Vec<String> = Vec::new();
    let mut done = BTreeSet::new();
    while ordered.len() < concepts.len() {
        let ready: Vec<_> = concepts
            .iter()
            .filter_map(|c| {
                let id = c["id"].as_str().unwrap();
                (!done.contains(id) && parents[id].iter().all(|p| done.contains(p)))
                    .then_some(id.to_string())
            })
            .collect();
        ensure!(!ready.is_empty(), "cyclic concept prerequisites");
        let group = ordered.last().and_then(|s| records[s]["group"].as_str());
        let selected = ready
            .iter()
            .find(|s| group.is_some() && records[*s]["group"].as_str() == group)
            .unwrap_or(&ready[0])
            .clone();
        done.insert(selected.clone());
        ordered.push(selected);
    }
    let weights = [
        ("conceptual", 0.15),
        ("mathematical", 0.30),
        ("algorithmic", 0.25),
        ("implementation", 0.40),
        ("research", 0.35),
        ("visual", 0.20),
        ("analytical", 0.20),
    ];
    let mut depths: BTreeMap<String, usize> = BTreeMap::new();
    let mut estimates = BTreeMap::new();
    let mut loads = BTreeMap::new();
    for id in &ordered {
        let c = records[id];
        let depth = parents[id].iter().map(|p| depths[p] + 1).max().unwrap_or(0);
        depths.insert(id.clone(), depth);
        let allocation = pedagogy::allocate_depth(c, "adaptive")?;
        let depth_load: f64 = weights
            .iter()
            .map(|(dim, w)| allocation["levels"][dim].as_u64().unwrap_or(0) as f64 * w)
            .sum();
        let components = json!({"concept":1.,"additional_objectives":0.25*(array(c,"objectives")?.len()-1)as f64,"prerequisite_depth":0.15*depth.min(6)as f64,"depth":depth_load,"worked_examples":0.25*list(c,"examples").len()as f64,"assessments":0.15*list(c,"assessments").len()as f64});
        let units = components
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_f64().unwrap())
            .sum::<f64>();
        loads.insert(id.clone(), units);
        estimates.insert(id.clone(),json!({"units":units,"components":components,"depth_levels":allocation["levels"],"prerequisite_depth":depth}));
    }
    let total: f64 = loads.values().sum();
    let desired = (total / 6.).ceil() as usize;
    let count = desired.clamp(5, 10).min(concepts.len());
    let target = total / count as f64;
    let n = ordered.len();
    let mut prefix = vec![0.];
    for id in &ordered {
        prefix.push(prefix.last().unwrap() + loads[id]);
    }
    let mut costs = vec![vec![f64::INFINITY; n + 1]; count + 1];
    let mut cuts = vec![vec![0; n + 1]; count + 1];
    costs[0][0] = 0.;
    for parts in 1..=count {
        for end in parts..=n - (count - parts) {
            for start in parts - 1..end {
                let boundary = start > 0
                    && records[&ordered[start]]["group"].as_str().is_some()
                    && records[&ordered[start]]["group"] == records[&ordered[start - 1]]["group"];
                let score = costs[parts - 1][start]
                    + ((prefix[end] - prefix[start] - target) / target).powi(2)
                    + if boundary { 0.2 } else { 0. };
                if score < costs[parts][end] {
                    costs[parts][end] = score;
                    cuts[parts][end] = start;
                }
            }
        }
    }
    let mut ranges = Vec::new();
    let mut end = n;
    for parts in (1..=count).rev() {
        let start = cuts[parts][end];
        ranges.push((start, end));
        end = start;
    }
    ranges.reverse();
    let mut chapters = Vec::new();
    let mut titles = BTreeSet::new();
    let mut group_owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (start, end) in ranges {
        let owned = &ordered[start..end];
        let groups: Vec<String> = owned
            .iter()
            .filter_map(|id| records[id]["group"].as_str().map(str::to_string))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let shared = groups.len() == 1
            && owned
                .iter()
                .all(|id| records[id]["group"].as_str() == Some(&groups[0]));
        let base = chapter_title(if shared {
            &groups[0]
        } else {
            records[&owned[0]]["title"].as_str().unwrap()
        })?;
        let mut title = base.clone();
        let mut i = 1;
        while !titles.insert(title.to_lowercase()) {
            i += 1;
            title = format!("{base} - Part {i}");
        }
        let identity = format!(
            "[{}]",
            owned
                .iter()
                .map(|s| serde_json::to_string(s).unwrap())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let identifier = format!("chapter-{}", &hash(identity.as_bytes())[..12]);
        let sections: Vec<_> = owned
            .iter()
            .map(|id| {
                records[id]["section"]
                    .as_str()
                    .unwrap_or(records[id]["title"].as_str().unwrap())
                    .to_string()
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for group in &groups {
            group_owners
                .entry(group.clone())
                .or_default()
                .push(identifier.clone());
        }
        chapters.push(json!({"id":identifier,"title":title,"concepts":owned,"sections":sections,"estimate":{"units":owned.iter().map(|s|loads[s]).sum::<f64>(),"concept_count":owned.len(),"groups":groups}}));
    }
    Ok(
        json!({"chapters":chapters,"rationale":{"method":"stable prerequisite order; workload-bounded count; balanced contiguous partition with group preference","chapter_count":count,"concept_count":n,"bounds":[5,10],"target_workload_units":6.,"balanced_workload_units":target,"total_workload_units":total,"unbounded_recommendation":desired,"concept_estimates":estimates,"topological_order":ordered,"external_prerequisites":external,"split_groups":group_owners.into_iter().filter(|(_,v)|v.len()>1).collect::<BTreeMap<_,_>>()}}),
    )
}
pub fn build_composition(spec: &Value) -> Result<Value> {
    ensure!(spec.is_object(), "composition specification must be object");
    let topic = text(spec, "topic")?;
    safe_title(topic)?;
    let policy = spec["source_policy"].as_str().unwrap_or("adaptive");
    ensure!(
        ["strict", "source_first", "adaptive"].contains(&policy),
        "invalid source policy"
    );
    let mut concepts = array(spec, "concepts")?.clone();
    let known = spec
        .get("known_prerequisites")
        .map(strings)
        .transpose()?
        .unwrap_or_default();
    let (parents, _) = inventory(&concepts, &known)?;
    let allocation = if spec.get("chapters").is_none() {
        plan_chapters(&concepts, &known)?
    } else {
        json!({"chapters":spec["chapters"],"rationale":{"method":"explicit author allocation","reason":spec["chapter_rationale"]}})
    };
    let mut chapters = array(&allocation, "chapters")?.clone();
    ensure!(
        (5..=10).contains(&chapters.len()),
        "comprehensive output requires 5-10 substantive chapters plus separate MOC"
    );
    let folder = format!("6 - Full Notes/{topic}");
    let mut owners = BTreeMap::new();
    let mut chapter_ids = BTreeSet::new();
    let mut done = BTreeSet::new();
    for (i, ch) in chapters.iter_mut().enumerate() {
        let id = text(ch, "id")?.to_string();
        ensure!(chapter_ids.insert(id.clone()), "duplicate chapter identity");
        let title = safe_title(text(ch, "title")?)?;
        let owned = strings(&ch["concepts"])?;
        ensure!(!owned.is_empty(), "chapter needs owned concepts");
        for cid in owned {
            ensure!(
                parents.contains_key(&cid) && owners.insert(cid.clone(), id.clone()).is_none(),
                "unknown or multiply owned concept"
            );
            ensure!(
                parents[&cid].iter().all(|p| done.contains(p)),
                "forward or cyclic prerequisite"
            );
            done.insert(cid);
        }
        ch["path"] = json!(format!("{folder}/{:02} - {title}.md", i + 1));
    }
    ensure!(
        owners.len() == concepts.len(),
        "every concept needs exactly one chapter owner"
    );
    let capabilities = pedagogy::capabilities()?;
    let mut obligations = Vec::new();
    for c in &mut concepts {
        let cid = text(c, "id")?.to_string();
        let selected = c
            .get("capabilities")
            .map(strings)
            .transpose()?
            .unwrap_or_else(|| vec!["S01".into()]);
        let selected = pedagogy::resolve_capabilities(&selected)?;
        c["capabilities"] = json!(selected);
        c["depth_allocation"] = pedagogy::allocate_depth(c, policy)?;
        c["objective_ids"] = json!(
            (0..array(c, "objectives")?.len())
                .map(|i| format!("{cid}:objective:{i}"))
                .collect::<Vec<_>>()
        );
        c["prerequisite_records"]=json!(list(c,"prerequisites").iter().map(|d|if d.is_object(){d.clone()}else{json!({"id":d,"classification":if parents.contains_key(d.as_str().unwrap_or("")){"covered"}else{"assumed"}})}).collect::<Vec<_>>());
        c["prerequisites"] = json!(parents[&cid]);
        let chapter = owners[&cid].clone();
        c["chapter"] = json!(chapters.iter().position(|ch| ch["id"] == chapter).unwrap() + 1);
        let mut research = Vec::new();
        for cap in &selected {
            for (i, requirement) in list(&capabilities[cap], "evidence_requirements")
                .iter()
                .enumerate()
            {
                research.push(json!({"id":format!("{cid}:{cap}:{i}"),"concept":cid,"capability":cap,"requirement":requirement,"source_policy":policy,"status":"planned","exact_evidence":true,"research_depth":c["depth_allocation"]["levels"]["research"],"primary_required":cap=="S06","freshness_required":cap=="S04"}));
            }
        }
        c["research_obligations"] = json!(research);
        for (i, goal) in array(c, "objectives")?.iter().enumerate() {
            obligations.push(json!({"id":format!("{cid}:objective:{i}"),"concept":cid,"owner":chapter,"deliverable":goal,"verifier":"review","severity":"critical","status":"planned"}));
        }
        for dim in pedagogy::DIMENSIONS {
            if c["depth_allocation"]["levels"][dim].as_u64().unwrap_or(0) > 0 {
                obligations.push(json!({"id":format!("{cid}:depth:{dim}"),"concept":cid,"owner":chapter,"verifier":"review","severity":"critical","deliverable":format!("Deliver independently allocated {dim} depth"),"status":"planned"}));
            }
        }
        for record in research {
            obligations.push(json!({"id":record["id"],"concept":cid,"owner":chapter,"verifier":"research_record","severity":"critical","record":record,"status":"planned"}));
        }
        for (field, verifier) in [
            ("proofs", "proof"),
            ("claims", "claim"),
            ("information_flows", "flow"),
            ("tensors", "tensor"),
            ("examples", "example"),
            ("implementations", "implementation"),
            ("numerical_tests", "numerical"),
            ("failures", "failure"),
            ("costs", "cost"),
            ("diagnostics", "diagnostics"),
            ("gaps", "gap"),
        ] {
            for record in list(c, field) {
                text(&record, "id")?;
                obligations.push(json!({"id":record["id"],"concept":cid,"owner":chapter,"verifier":verifier,"record":record,"severity":"critical","status":"planned"}));
            }
        }
        for (dim, field) in [
            ("mathematical", "proofs"),
            ("implementation", "implementations"),
        ] {
            if c["depth_allocation"]["levels"][dim].as_u64().unwrap_or(0) >= 3
                && list(c, field).is_empty()
            {
                obligations.push(json!({"id":format!("{cid}:required-{field}"),"concept":cid,"owner":chapter,"verifier":"missing","deliverable":format!("Supply the consequential {field} contract"),"severity":"critical","status":"planned"}));
            }
        }
    }
    let assessments = list(spec, "assessments");
    let objective_owner: BTreeMap<String, String> = concepts
        .iter()
        .flat_map(|c| {
            list(c, "objective_ids")
                .into_iter()
                .map(|id| {
                    (
                        id.as_str().unwrap().into(),
                        owners[c["id"].as_str().unwrap()].clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut question_ids = BTreeSet::new();
    for q in &assessments {
        let id = text(q, "id")?;
        ensure!(question_ids.insert(id), "duplicate assessment identity");
        let ch = text(q, "chapter")?;
        ensure!(chapter_ids.contains(ch), "assessment has unknown chapter");
        ensure!(
            !array(q, "objectives")?.is_empty(),
            "assessment needs objectives"
        );
        for oid in strings(&q["objectives"])? {
            ensure!(
                objective_owner.get(&oid).is_some_and(|owner| owner == ch),
                "assessment objective belongs to another chapter"
            );
        }
        assessment::render_question(q)?;
    }
    for ch in &chapters {
        obligations.push(json!({"id":format!("{}:assessment",text(ch,"id")?),"owner":ch["id"],"verifier":"assessment","severity":"critical","record":ch,"status":"planned"}));
    }
    let mut ids = BTreeSet::new();
    for o in &obligations {
        ensure!(ids.insert(text(o, "id")?), "duplicate obligation identity");
    }
    if let Some(notation) = spec.get("notation") {
        ensure!(
            notation
                .as_object()
                .is_some_and(|m| m.iter().all(|(k, v)| !k.trim().is_empty()
                    && v.as_str().is_some_and(|s| !s.trim().is_empty()))),
            "notation must map symbols to definitions"
        );
    }
    Ok(
        json!({"version":2,"output_profile":"comprehensive_multifile","topic":topic,"source_policy":policy,"folder":folder,"moc_path":format!("{folder}/00 - Map of Contents.md"),"chapters":chapters,"concepts":concepts,"chapter_rationale":allocation["rationale"],"classification":spec["classification"],"notation":spec.get("notation").cloned().unwrap_or(json!({})),"visuals":list(spec,"visuals"),"tables":list(spec,"tables"),"assessments":assessments,"obligations":obligations,"spec_hash":hash(&serde_json::to_vec(spec)?),"validator_version":"rust-native-v1","review_required":["factual_accuracy","derivation_correctness","reader_simulation","cross_chapter_coherence"]}),
    )
}
fn visual_renderer(visual: &Value) -> Result<&str> {
    if let Some(renderer) = visual["renderer"].as_str() {
        return Ok(renderer);
    }
    Ok(
        match visual["kind"]
            .as_str()
            .context("visual needs semantic kind or renderer")?
        {
            "equation" | "distribution" | "signal" | "timeseries" => "scientific",
            "geometry" | "spatial" => "coordinates",
            "network" | "circuit" | "algorithm" | "chronology" | "research" | "workflow"
            | "dependency" | "state" => "graph",
            "tensor" | "architecture" => "svg",
            "mask" => "mask",
            "source_figure" => "external",
            "table" => "table",
            "interactive" => "interactive",
            "intuition" => "imagegen",
            _ => bail!("unsupported visual semantic kind"),
        },
    )
}
fn visual_identity(visual: &Value, index: usize) -> String {
    visual["id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "{}:visual:{index}",
                visual["concept"].as_str().unwrap_or("unknown")
            )
        })
}
fn visual_owner<'a>(plan: &'a Value, visual: &Value) -> Result<&'a Value> {
    let concept = text(visual, "concept")?;
    array(plan, "chapters")?
        .iter()
        .find(|ch| {
            ch["concepts"]
                .as_array()
                .is_some_and(|a| a.iter().any(|c| c == concept))
        })
        .context("visual has no owning chapter")
}
pub fn to_visual_plan(plan: &Value) -> Result<crate::planning::KnowledgePlan> {
    let mut visuals = Vec::new();
    let mut textual = Vec::new();
    for (i, v) in list(plan, "visuals").iter().enumerate() {
        let renderer = visual_renderer(v)?;
        let id = visual_identity(v, i);
        let owner = visual_owner(plan, v)?;
        if renderer == "table" {
            textual.push(json!({"id":id,"kind":"structured_table","placement":owner["path"],"verification":"exact deterministic table text in owned section"}));
            continue;
        }
        let category = match renderer {
            "scientific" => "numerical_plot",
            "coordinates" => "spatial",
            "graph" => "graph",
            "mask" => "heatmap",
            "external" => "research_figure",
            _ => "architecture",
        };
        let family = match v["family"].as_str().unwrap_or("R5") {
            "R1" | "mathematical-geometric" => "mathematical_geometric",
            "R2" | "scientific-statistical" => "scientific_statistical",
            "R3" | "engineering-physical" => "engineering_systems",
            "R4" | "ai-ml" => "ai_ml",
            "R6" | "data-analytics" => "data_analytics",
            "R7" | "graphs-networks" => "knowledge_graphs",
            "R8" | "spatial-structural" => "spatial_structures",
            "R9" | "research-evidence" => "research_evidence",
            "R10" | "interactive-simulation" => "interactive",
            "R11" | "external-media" => "external_media",
            "R12" | "structured-tables" => "structured_tables",
            _ => "computer_science",
        };
        visuals.push(json!({"id":id,"concept_id":text(v,"concept")?,"objective":text(v,"objective")?,"category":category,"description":v["description"].as_str().unwrap_or(text(v,"objective")?),"required_information":v.get("required_information").cloned().unwrap_or(json!([text(v,"objective")?])),"renderer":renderer,"family":family,"verification_method":"Validate semantic renderer inputs; inspect decoded local bytes and hash; verify exact owned embed","placement":owner["path"],"required":v["required"].as_bool().unwrap_or(true)}));
    }
    let concepts:Vec<_>=array(plan,"concepts")?.iter().map(|c|json!({"id":c["id"],"title":c["title"],"objectives":c["objectives"],"prerequisites":c["prerequisites"],"complexity":c["complexity"].as_u64().unwrap_or(3)})).collect();
    let notes: Vec<_> = array(plan, "chapters")?
        .iter()
        .map(|ch| ch["path"].clone())
        .chain(std::iter::once(plan["moc_path"].clone()))
        .collect();
    crate::planning::build_plan(serde_json::from_value(
        json!({"topic":plan["topic"],"source_policy":plan["source_policy"],"concepts":concepts,"notes":notes,"visuals":visuals,"textual_visuals":textual}),
    )?)
}
pub async fn prepare_visuals(
    vault: &std::path::Path,
    plan: &Value,
    drafts: &BTreeMap<String, String>,
) -> Result<(
    BTreeMap<String, String>,
    Vec<crate::validation::VisualReceipt>,
)> {
    use crate::{
        assets::{AssetRecord, AssetStore, Provenance},
        config::Config,
        network,
        planning::VisualState,
        validation::{VisualReceipt, safe_path},
    };
    let mut bodies = drafts.clone();
    let mut receipts = Vec::new();
    let config = Config::default();
    for (i, v) in list(plan, "visuals").iter().enumerate() {
        let owner = visual_owner(plan, v)?;
        let owner_id = text(owner, "id")?;
        let Some(body) = bodies.get_mut(owner_id) else {
            bail!("visual owner lacks authored chapter: {owner_id}")
        };
        ensure!(!body.trim().is_empty(), "visual owner chapter is empty");
        let section = text(v, "section")?;
        section_body(body, section)?;
        let mut spec = v.clone();
        let renderer = visual_renderer(v)?;
        spec["renderer"] = json!(renderer);
        let objective = text(v, "objective")?;
        if renderer == "table" {
            let artifact = crate::rendering::render(&spec)?;
            let table = String::from_utf8(artifact.bytes)?;
            if !section_body(body, section)?.contains(&table) {
                *body = insert_at_section(body, section, &table)?;
            }
            continue;
        }
        let note = std::path::Path::new(text(owner, "path")?);
        let store = AssetStore::new(vault, config.attachment_dir(vault, note)?)?;
        let record: AssetRecord = if let Some(record) = v.get("asset_record") {
            let record: AssetRecord = serde_json::from_value(record.clone())?;
            store.verify(&record)?;
            record
        } else if renderer == "external" {
            if plan["source_policy"] == "strict" {
                ensure!(
                    v["origin"] == "supplied",
                    "strict source mode requires supplied visual"
                );
            }
            let source = text(v, "source")?.to_string();
            let license = v["license"]
                .as_str()
                .or_else(|| v["rights"].as_str())
                .context("source image needs license or rights")?
                .to_string();
            let mut provenance = Provenance {
                source_page: source,
                image_url: String::new(),
                creator: text(v, "creator")?.into(),
                license,
                permission_basis: text(v, "permission_basis")?.into(),
                relevance: text(v, "relevance")?.into(),
                original_filename: v["original_filename"]
                    .as_str()
                    .unwrap_or("source-figure.png")
                    .into(),
                retrieved_at: crate::cache::now()?,
                final_url: None,
            };
            if let Some(local) = v["local_fallback"].as_str() {
                let path = safe_path(vault, std::path::Path::new(local), true)?;
                ensure!(
                    std::fs::metadata(&path)?.len() <= crate::assets::MAX_BYTES as u64,
                    "source image too large"
                );
                let mime = match path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .as_str()
                {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "webp" => "image/webp",
                    _ => bail!("unsupported supplied image format"),
                };
                store.store_bytes(&std::fs::read(path)?, mime, provenance)?
            } else {
                ensure!(
                    v["asset_policy"] != "offline_only",
                    "offline-only visual requires supplied local image"
                );
                let url = text(v, "url")?;
                provenance.image_url = url.into();
                let downloaded =
                    network::fetch_image(url, &network::DownloadPolicy::default()).await?;
                provenance.final_url = Some(downloaded.final_url);
                store.store_bytes(&downloaded.bytes, &downloaded.mime, provenance)?
            }
        } else {
            let artifact = crate::rendering::render(&spec)?;
            ensure!(
                artifact.mime == "image/svg+xml",
                "required visual needs a host renderer for {} output; source emission does not certify image fulfillment",
                artifact.mime
            );
            store.store_generated(&artifact, objective)?
        };
        store.verify(&record)?;
        let embed = store.embed(&record, v["alt"].as_str().unwrap_or(objective))?;
        if !section_body(body, section)?.contains(&embed) {
            *body = insert_at_section(body, section, &embed)?;
        }
        receipts.push(VisualReceipt {
            visual_id: visual_identity(v, i),
            local_path: record.local_path,
            sha256: record.sha256,
            provenance: serde_json::to_value(record.provenance)?,
            state: VisualState::Verified,
        });
    }
    Ok((bodies, receipts))
}
fn headings(body: &str) -> Vec<(usize, usize, String)> {
    let mut offset = 0;
    let mut fence: Option<char> = None;
    let mut out = Vec::new();
    for line in body.split_inclusive('\n') {
        let clean = line.trim_end();
        let trimmed = clean.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = trimmed.chars().next().unwrap();
            if fence == Some(marker) {
                fence = None
            } else if fence.is_none() {
                fence = Some(marker)
            }
        } else if fence.is_none() {
            let n = clean.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&n) && clean.as_bytes().get(n) == Some(&b' ') {
                out.push((offset, n, clean[n..].trim().into()));
            }
        }
        offset += line.len();
    }
    out
}
fn section_range(body: &str, name: &str) -> Result<(usize, usize)> {
    let headings = headings(body);
    let mut matches: Vec<_> = headings
        .iter()
        .enumerate()
        .filter(|(_, (_, level, title))| title == name && *level > 1)
        .collect();
    if matches.is_empty() {
        matches = headings
            .iter()
            .enumerate()
            .filter(|(_, (_, _, title))| title == name)
            .collect();
    }
    ensure!(
        matches.len() == 1,
        "missing or ambiguous contribution placement heading: {name}"
    );
    let (i, (start, level, _)) = matches[0];
    let end = headings[i + 1..]
        .iter()
        .find(|(_, next, _)| next <= level)
        .map(|(at, _, _)| *at)
        .unwrap_or(body.len());
    Ok((*start, end))
}
pub fn section_body<'a>(body: &'a str, name: &str) -> Result<&'a str> {
    let (start, end) = section_range(body, name)?;
    Ok(&body[start..end])
}
pub fn insert_at_section(body: &str, section: &str, contribution: &str) -> Result<String> {
    let (_, end) = section_range(body, section)?;
    Ok(format!(
        "{}\n\n{contribution}\n\n{}",
        body[..end].trim_end(),
        &body[end..]
    ))
}
pub fn prepare_drafts(
    plan: &Value,
    drafts: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let mut result = drafts.clone();
    for ch in array(plan, "chapters")? {
        let id = text(ch, "id")?;
        let Some(body) = result.get_mut(id) else {
            continue;
        };
        if body.trim().is_empty() {
            continue;
        }
        let owned = strings(&ch["concepts"])?;
        for table in list(plan, "tables") {
            if owned
                .iter()
                .any(|s| Some(s.as_str()) == table["concept"].as_str())
            {
                let mut spec = table.clone();
                spec["renderer"] = json!("table");
                if spec.get("objective").is_none() {
                    spec["objective"] = json!("Compare declared educational records");
                }
                let rendered = crate::rendering::render(&spec)?;
                let rendered = String::from_utf8(rendered.bytes)?;
                if !body.contains(&rendered) {
                    *body = insert_at_section(body, text(&table, "section")?, &rendered)?;
                }
            }
        }
        let mut final_questions = Vec::new();
        for q in list(plan, "assessments") {
            if q["chapter"] != ch["id"] {
                continue;
            }
            let rendered = assessment::render_question(&q)?;
            if body.contains(&rendered) {
                continue;
            }
            let placement = q["placement"].as_str().unwrap_or("chapter_end");
            if placement == "chapter_end" {
                final_questions.push(rendered)
            } else {
                *body = insert_at_section(body, placement, &rendered)?;
            }
        }
        if !final_questions.is_empty() {
            *body = format!(
                "{}\n\n## Check your understanding\n\n{}\n",
                body.trim_end(),
                final_questions.join("\n\n")
            );
        }
    }
    Ok(result)
}
pub fn render_moc(plan: &Value) -> Result<String> {
    let mut lines = vec![
        format!("# {}", text(plan, "topic")?),
        String::new(),
        "## Learning path".into(),
        String::new(),
    ];
    for (i, ch) in array(plan, "chapters")?.iter().enumerate() {
        let stem = std::path::Path::new(text(ch, "path")?)
            .file_stem()
            .and_then(|s| s.to_str())
            .context("chapter path")?;
        lines.push(format!("{}. [[{stem}]]", i + 1));
    }
    lines.extend([
        String::new(),
        "## Concept prerequisites".into(),
        String::new(),
        "```mermaid".into(),
        "flowchart TD".into(),
    ]);
    let concepts = array(plan, "concepts")?;
    let ids: BTreeMap<_, _> = concepts
        .iter()
        .enumerate()
        .map(|(i, c)| (c["id"].as_str().unwrap(), format!("n{i}")))
        .collect();
    for c in concepts {
        let id = text(c, "id")?;
        let label = text(c, "title")?
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;");
        lines.push(format!("    {}[\"{label}\"]", ids[id]));
        for dep in list(c, "prerequisites") {
            if let Some(parent) = dep.as_str().and_then(|s| ids.get(s)) {
                lines.push(format!("    {parent} --> {}", ids[id]));
            }
        }
    }
    lines.extend(["```".into(), String::new()]);
    Ok(lines.join("\n"))
}
pub fn five_key_note(
    plan: &Value,
    id: &str,
    title: &str,
    body: &str,
    next: Option<&str>,
) -> Result<String> {
    let domain = plan["classification"]["domain"]
        .as_str()
        .unwrap_or("general");
    let subject = plan["classification"]["subject"]
        .as_str()
        .or_else(|| plan["classification"]["master_topic"].as_str())
        .unwrap_or(text(plan, "topic")?);
    let props = json!({"id":id,"aliases":[title],"topic":plan["topic"],"tags":[format!("topic/{}",slug(text(plan,"topic")?)),format!("subject/{}",slug(subject)),format!("domain/{}",slug(domain)),"type/full-notes"],"nexttopics":next.map(|s|vec![format!("[[{s}]]")]).unwrap_or_default()});
    Ok(format!(
        "---\n{}---\n{}\n",
        serde_yaml_ng::to_string(&props)?,
        body.trim()
    ))
}
pub fn compose_documents(
    plan: &Value,
    drafts: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let mut docs = BTreeMap::new();
    let chapters = array(plan, "chapters")?;
    for (i, ch) in chapters.iter().enumerate() {
        let id = text(ch, "id")?;
        let body = drafts.get(id).context("missing authored chapter")?;
        ensure!(!body.trim().is_empty(), "empty authored chapter");
        let next = chapters
            .get(i + 1)
            .and_then(|v| v["path"].as_str())
            .and_then(|s| std::path::Path::new(s).file_stem())
            .and_then(|s| s.to_str());
        docs.insert(
            text(ch, "path")?.into(),
            five_key_note(plan, id, text(ch, "title")?, body, next)?,
        );
    }
    let first = chapters
        .first()
        .and_then(|c| c["path"].as_str())
        .and_then(|s| std::path::Path::new(s).file_stem())
        .and_then(|s| s.to_str());
    docs.insert(
        text(plan, "moc_path")?.into(),
        five_key_note(
            plan,
            &format!("{}.moc", slug(text(plan, "topic")?)),
            "Map of Contents",
            &render_moc(plan)?,
            first,
        )?,
    );
    Ok(docs)
}
pub fn validate_chapter(body: &str, required_sections: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    let lower = body.to_lowercase();
    if [
        "todo",
        "tbd",
        "fixme",
        "[placeholder]",
        "<!-- scaffold",
        "<!-- temporary",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        errors.push("unfinished content".into());
    }
    let mut fence = None;
    let mut prose = String::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = trimmed.chars().next().unwrap();
            if fence == Some(marker) {
                fence = None
            } else if fence.is_none() {
                fence = Some(marker)
            }
        } else if fence.is_none() {
            prose.push_str(line);
            prose.push('\n');
        }
    }
    if fence.is_some() {
        errors.push("unclosed code fence".into());
    }
    if !prose.matches("$$").count().is_multiple_of(2) {
        errors.push("unclosed display mathematics".into());
    }
    let sections = headings(body);
    for section in required_sections {
        if !sections.iter().any(|(_, _, s)| s == section) {
            errors.push(format!("missing planned section: {section}"));
        }
    }
    errors
}
pub fn validate_source_snapshots(vault: &std::path::Path, plan: &Value) -> Result<Vec<String>> {
    let mut errors = Vec::new();
    for concept in array(plan, "concepts")? {
        for claim in list(concept, "claims") {
            let source = &claim["source"];
            if let Some(snapshot) = source["snapshot"].as_str() {
                let check: Result<()> = (|| {
                    let path =
                        crate::validation::safe_path(vault, std::path::Path::new(snapshot), true)?;
                    ensure!(
                        std::fs::metadata(&path)?.len() <= 64 * 1024 * 1024,
                        "source snapshot too large"
                    );
                    let bytes = std::fs::read(path)?;
                    ensure!(
                        source["hash"] == hash(&bytes),
                        "source snapshot hash does not bind provenance"
                    );
                    let text = std::str::from_utf8(&bytes)?.trim_start_matches('\u{feff}');
                    let excerpt = source["excerpt"]
                        .as_str()
                        .context("source snapshot needs exact excerpt")?;
                    ensure!(
                        !excerpt.is_empty() && text.contains(excerpt),
                        "source snapshot does not support excerpt"
                    );
                    Ok(())
                })();
                if let Err(e) = check {
                    errors.push(format!("{}: {e}", claim["id"].as_str().unwrap_or("claim")));
                }
            }
        }
    }
    Ok(errors)
}
pub fn evidence_status(obligations: &[Value], evidence: &Value) -> Value {
    let mut pending = Vec::new();
    let mut escalate = Vec::new();
    for o in obligations {
        let id = o["id"].as_str().unwrap_or("");
        let r = &evidence[id];
        let mut valid = r["verified"] == true
            && ["source", "location"]
                .iter()
                .all(|s| r.get(s).is_some_and(|v| !v.is_null() && v != ""))
            && r["contradictions"].as_array().is_none_or(Vec::is_empty);
        if o["exact_evidence"] == true {
            valid &= [
                "claim",
                "classification",
                "version",
                "verification_method",
                "limits",
            ]
            .iter()
            .all(|s| r.get(s).is_some_and(|v| !v.is_null() && v != ""));
        }
        if o["source_policy"] == "strict" {
            valid &= r["origin"] == "supplied";
        }
        if o["primary_required"] == true {
            valid &= r["primary"] == true;
        }
        if o["freshness_required"] == true {
            valid &= r.get("checked_at").is_some() && r.get("version").is_some();
        }
        if !valid {
            pending.push(id);
        }
        if r["contradictions"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            escalate.push(id);
        }
    }
    json!({"sufficient":pending.is_empty(),"stop_retrieval":pending.is_empty(),"pending":pending,"escalate":escalate})
}
pub fn receipt_errors(receipt: &Value, drafts: &BTreeMap<String, String>) -> Vec<String> {
    let mut errors = Vec::new();
    if !receipt.is_object() {
        return vec!["review must be structured artifact evidence".into()];
    }
    for key in ["verifier", "method", "limitations", "result"] {
        if !receipt[key].as_str().is_some_and(|s| !s.trim().is_empty()) {
            errors.push(format!("review missing {key}"));
        }
    }
    if receipt["result"] != "pass" {
        errors.push("review did not pass".into());
    }
    let Some(items) = receipt["evidence"].as_array() else {
        errors.push("review must bind exact chapter content".into());
        return errors;
    };
    if items.is_empty() {
        errors.push("review evidence cannot be empty".into());
    }
    for item in items {
        let body = item["chapter"].as_str().and_then(|id| drafts.get(id));
        if body.is_none_or(|b| {
            item["hash"] != hash(b.as_bytes())
                || !item["excerpt"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && b.contains(s))
        }) {
            errors.push("stale or ungrounded review evidence".into());
        }
    }
    errors
}
pub fn validate_composition(
    plan: &Value,
    drafts: &BTreeMap<String, String>,
    evidence: &Value,
    reviews: &Value,
) -> Result<Value> {
    let mut errors = Vec::new();
    let mut states = Vec::new();
    for ch in array(plan, "chapters")? {
        let id = text(ch, "id")?;
        let body = drafts.get(id).map(String::as_str).unwrap_or("");
        if body.trim().is_empty() {
            errors.push(format!("missing authored chapter: {id}"));
        }
        let owned = strings(&ch["concepts"])?;
        let mut sections = ch
            .get("sections")
            .map(strings)
            .transpose()?
            .unwrap_or_default();
        for c in array(plan, "concepts")? {
            if owned.iter().any(|s| Some(s.as_str()) == c["id"].as_str()) {
                sections.push(c["section"].as_str().unwrap_or(text(c, "title")?).into());
            }
        }
        for e in validate_chapter(body, &sections) {
            errors.push(format!("{id}: {e}"));
        }
        for e in assessment::check_assessment(plan, ch, body) {
            errors.push(format!("{id}: {e}"));
        }
    }
    for name in [
        "factual_accuracy",
        "derivation_correctness",
        "reader_simulation",
        "cross_chapter_coherence",
    ] {
        let r = &reviews[name];
        for e in receipt_errors(r, drafts) {
            errors.push(format!("{name}: {e}"));
        }
        let covered: BTreeSet<_> = r["evidence"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["chapter"].as_str())
            .collect();
        for ch in array(plan, "chapters")? {
            if !covered.contains(text(ch, "id")?) {
                errors.push(format!("{name}: missing chapter review"));
            }
        }
    }
    for o in array(plan, "obligations")? {
        let id = text(o, "id")?;
        let body = o["owner"]
            .as_str()
            .and_then(|id| drafts.get(id))
            .map(String::as_str)
            .unwrap_or("");
        let r = &o["record"];
        let mut failures = Vec::new();
        match o["verifier"].as_str().unwrap_or("") {
            "review" | "assessment" => (),
            "missing" => failures.push(
                o["deliverable"]
                    .as_str()
                    .unwrap_or("missing required technical contract")
                    .into(),
            ),
            "research_record" => {
                if evidence_status(std::slice::from_ref(r), evidence)["sufficient"] != true {
                    failures.push("insufficient exact source evidence".into());
                }
            }
            "numerical" => failures.extend(pedagogy::check_numerical(r)),
            "flow" => {
                if let Err(e) = crate::rendering::validate_flow(r) {
                    failures.push(e.to_string());
                }
            }
            "gap" => {
                let bridge = r["resolved_excerpt"].as_str().unwrap_or("");
                if bridge.is_empty() || !body.contains(bridge) {
                    failures.push("explanation gap lacks actual prerequisite bridge".into());
                }
                if let Some(before) = r["before"].as_str()
                    && body.find(bridge) >= body.find(before)
                {
                    failures.push("prerequisite bridge must precede dependent term".into());
                }
            }
            "proof" | "claim" | "tensor" | "example" | "implementation" | "failure" | "cost"
            | "diagnostics" => {
                failures.extend(check_technical_record(
                    o["verifier"].as_str().unwrap(),
                    r,
                    body,
                    plan["source_policy"].as_str().unwrap_or("adaptive"),
                ));
            }
            other => failures.push(format!("unimplemented verifier contract: {other}")),
        }
        if [
            "flow",
            "tensor",
            "example",
            "numerical",
            "failure",
            "cost",
            "diagnostics",
        ]
        .contains(&o["verifier"].as_str().unwrap_or(""))
            && !r["artifact_excerpt"]
                .as_str()
                .is_some_and(|s| !s.is_empty() && body.contains(s))
        {
            failures.push("technical contract lacks actual educational artifact excerpt".into());
        }
        for failure in &failures {
            errors.push(format!("{id}: {failure}"));
        }
        states.push(json!({"id":id,"status":if failures.is_empty(){"verified"}else{"blocked"},"errors":failures}));
    }
    Ok(
        json!({"complete":errors.is_empty(),"errors":errors,"obligations":states,"review_scope":"Exact artifact-bound assertions; deterministic checks do not establish scientific truth"}),
    )
}
fn check_technical_record(kind: &str, r: &Value, body: &str, policy: &str) -> Vec<String> {
    let result: Result<()> = (|| {
        match kind {
            "claim" => {
                pedagogy::check_claim(r, policy)?;
                ensure!(
                    body.contains(text(r, "text")?),
                    "exact claim absent from chapter"
                );
            }
            "example" => pedagogy::check_example(r, body)?,
            "implementation" => pedagogy::check_implementation(r, body)?,
            "failure" => pedagogy::check_failure(r)?,
            "cost" => pedagogy::check_cost(r)?,
            "diagnostics" => pedagogy::check_diagnostics(r)?,
            "tensor" => {
                for key in [
                    "name",
                    "shape",
                    "dtype",
                    "role",
                    "valid_range",
                    "gradient_owner",
                    "indexing",
                    "padding",
                    "boundaries",
                    "tokenizer",
                    "label_shift",
                    "readout",
                ] {
                    ensure!(r.get(key).is_some(), "tensor missing {key}");
                }
                let dims: Vec<u64> = array(r, "shape")?
                    .iter()
                    .map(|v| {
                        v.as_u64()
                            .or_else(|| v.as_str().and_then(|s| r["dimensions"][s].as_u64()))
                            .context("unresolved tensor dimension")
                    })
                    .collect::<Result<_>>()?;
                ensure!(
                    ["zero_based", "one_based"].contains(&text(r, "indexing")?),
                    "unknown tensor indexing"
                );
                if !r["readout"].is_null() {
                    let axis = r["readout"]["axis"]
                        .as_u64()
                        .context("readout axis missing")? as usize;
                    let pos = r["readout"]["position"]
                        .as_u64()
                        .context("readout position missing")?;
                    let offset = if r["indexing"] == "one_based" { 1 } else { 0 };
                    ensure!(
                        dims.get(axis)
                            .is_some_and(|size| pos >= offset && pos < size + offset),
                        "tensor readout out of bounds"
                    );
                }
                if r["schematic_tokens"] == true && r["tokenizer"] != "schematic" {
                    ensure!(
                        r.get("tokenization_evidence").is_some(),
                        "schematic tokens cannot assert real tokenizer boundaries"
                    );
                }
            }
            "proof" => {
                for key in [
                    "claim",
                    "assumptions",
                    "domain",
                    "steps",
                    "scope",
                    "approximations",
                    "boundary_conditions",
                ] {
                    ensure!(r.get(key).is_some(), "proof missing {key}");
                }
                ensure!(
                    body.contains(text(r, "claim")?),
                    "proof claim absent from chapter"
                );
                ensure!(
                    !(r["verification"] == "numerical_samples" && r["universal"] != false),
                    "numerical samples are not universal proof"
                );
                ensure!(
                    !(r["scope"] == "finite_update" && r["justification_scope"] == "first_order"),
                    "first-order geometry cannot prove finite-step preservation"
                );
                let assumptions = strings(&r["assumptions"])?;
                for a in list(r, "conclusion_assumptions") {
                    ensure!(
                        assumptions.iter().any(|s| a == s.as_str()),
                        "conclusion requires unstated assumptions"
                    );
                }
                ensure!(
                    !array(r, "steps")?.is_empty(),
                    "proof needs intermediate steps"
                );
                for step in array(r, "steps")? {
                    text(step, "reason")?;
                    match text(step, "relation")? {
                        "identity" => ensure!(
                            pedagogy::exact_identity(text(step, "lhs")?, text(step, "rhs")?)?,
                            "exact algebraic identity fails"
                        ),
                        "approximation" | "implication" => {
                            ensure!(
                                !array(step, "assumptions")?.is_empty(),
                                "nonidentity step needs assumptions"
                            );
                            text(step, "justification")?;
                        }
                        _ => bail!("unknown proof relation"),
                    }
                }
                for check in list(r, "checks") {
                    let errors = pedagogy::check_numerical(&check);
                    ensure!(errors.is_empty(), "{}", errors.join("; "));
                }
            }
            _ => bail!("unknown technical verifier"),
        }
        Ok(())
    })();
    result
        .err()
        .map(|e| vec![e.to_string()])
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn concepts() -> Vec<Value> {
        (0..12).map(|i|json!({"id":format!("c{i}"),"title":format!("Concept {i}"),"objectives":["Explain a mechanism"],"prerequisites":if i>0{vec![format!("c{}",i-1)]}else{vec![]}})).collect()
    }
    #[test]
    fn adaptive_partition_is_stable_and_prerequisite_ordered() {
        let a = plan_chapters(&concepts(), &[]).unwrap();
        assert!((5..=10).contains(&a["chapters"].as_array().unwrap().len()));
        assert_eq!(a, plan_chapters(&concepts(), &[]).unwrap());
        assert_eq!(a["rationale"]["topological_order"][11], "c11");
        let mut cycle = concepts();
        cycle[0]["prerequisites"] = json!(["c11"]);
        assert!(plan_chapters(&cycle, &[]).is_err());
    }
    #[test]
    fn narrow_topics_are_not_padded() {
        assert!(plan_chapters(&concepts()[..4], &[]).is_err());
    }
    #[test]
    fn placement_ignores_code_and_is_idempotent() {
        let b = "## Concept\nReal text\n```md\n## Fake\n```\n## Next\nOther";
        assert!(insert_at_section(b, "Fake", "table").is_err());
        let prepared = insert_at_section(b, "Concept", "table").unwrap();
        assert!(prepared.find("table").unwrap() < prepared.find("## Next").unwrap());
    }
    #[test]
    fn stale_reviews_and_unverified_sources_are_rejected() {
        let drafts = BTreeMap::from([("ch".into(), "Authored body".into())]);
        let receipt = json!({"verifier":"Reviewer","method":"read","limitations":"assertion","result":"pass","evidence":[{"chapter":"ch","hash":hash(b"old"),"excerpt":"Authored"}]});
        assert!(!receipt_errors(&receipt, &drafts).is_empty());
        assert_eq!(
            evidence_status(
                &[json!({"id":"claim","source_policy":"strict","exact_evidence":true})],
                &json!({})
            )["sufficient"],
            false
        );
    }
}
