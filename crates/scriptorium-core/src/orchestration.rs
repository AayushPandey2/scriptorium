//! Native request routing, explicit constraint precedence and scope decisions.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Mutex, OnceLock},
};

pub fn matches(pattern: &str, text: &str) -> bool {
    regexp(pattern).is_match(text)
}
fn regexp(pattern: &str) -> regex::Regex {
    static CACHE: OnceLock<Mutex<BTreeMap<String, regex::Regex>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap();
    cache
        .entry(pattern.into())
        .or_insert_with(|| {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .expect("fixed valid routing regex")
        })
        .clone()
}
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct RouteOptions {
    pub source_policy: Option<String>,
    pub has_source: bool,
    pub trace: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteDecision {
    pub command: String,
    pub mode: String,
    pub master: String,
    pub variant: Option<String>,
    pub compat_route: String,
    pub source_policy: String,
    pub research_mode: String,
    pub multi_artifact: bool,
    pub shared_context: bool,
    pub artifacts: Vec<Value>,
    pub knowledge_types: Vec<String>,
    pub routing_trace: Value,
    pub retired_command: Option<String>,
    pub migration_hint: Option<String>,
    pub topic: String,
    pub lock: Option<String>,
}
pub fn note_type(master: &str) -> &'static str {
    match master {
        "M2" => "study-notes",
        "M3" => "quick-reference",
        "M4" => "worked-solutions",
        "M5" => "implementation-guide",
        "M6" => "assessment",
        "M7" => "synthesis-revision",
        _ => "deep-notes",
    }
}
fn compatibility(master: &str, variant: Option<&str>) -> String {
    match variant {
        Some("formula_sheet") => "cheat-sheet",
        Some("syntax_reference") => "api-table",
        Some("atomic_card") => "atomic",
        Some("comparison_guide") => "comparison",
        Some("quick_summary") => "summary",
        Some("paper_breakdown") => "paper-breakdown",
        _ => match master {
            "M1" => "deepstudy-master",
            "M2" => "studynotes",
            "M3" => "cheat-sheet",
            "M5" => "dsa",
            "M6" => "practice",
            "M7" => "comparison",
            _ => note_type(master),
        },
    }
    .into()
}
fn signals(text: &str, has_source: bool, forbidden: &BTreeSet<String>) -> (String, Option<String>) {
    let rules = [
        (
            r"\b(formula|equation)[ -]?sheet\b",
            "M3",
            Some("formula_sheet"),
        ),
        (
            r"\b(syntax|api)[ -]?(reference|table|sheet|guide)\b|\blookup table\b",
            "M3",
            Some("syntax_reference"),
        ),
        (
            r"\b(command|cli)[ -]?(reference|sheet|guide)\b",
            "M3",
            Some("command_reference"),
        ),
        (r"\batomic[ -]?(note|card)\b", "M3", Some("atomic_card")),
        (
            r"\bcheat[ -]?sheet\b|\bquick[ -]?reference\b|\bcrisp reference\b",
            "M3",
            None,
        ),
        (
            r"\bflashcards?\b|\banki\s+cards?\b",
            "M6",
            Some("flashcards"),
        ),
        (
            r"\bmock[ -]?exam\b|\bviva\b|\boral[ -]?exam\b",
            "M6",
            Some("mock_exam"),
        ),
        (
            r"\b(practice[ -]?(test|set|problems?|questions?|exercises?)|quiz|self-test|test myself|active[ -]?recall)\b",
            "M6",
            None,
        ),
        (
            r"\b(comprehensive|deep|detailed|first[ -]?principles)\s+(study\s+)?notes?\b",
            "M1",
            None,
        ),
        (
            r"\b(worked[ -]?(solutions?|examples?|problems?)|step-by-step solutions?|solve\b.*\b(problems?|exercises?|integrals?|equations?))\b",
            "M4",
            None,
        ),
        (
            r"\b(implement(ation)?|debug(ging)?|troubleshoot(ing)?|deployment guide|code walkthrough|build\s+(a|an|the))\b",
            "M5",
            None,
        ),
        (
            r"\bcompare\b|\bvs\.?\b|\bversus\b|\btrade-?offs? between\b|\bcomparison\b",
            "M7",
            Some("comparison_guide"),
        ),
        (
            r"\bpaper[ -]?breakdown\b|\bbreak[ -]?down (this|the) paper\b",
            "M7",
            Some("paper_breakdown"),
        ),
        (
            r"\b(concept|knowledge|relationship)[ -]?map\b",
            "M7",
            Some("concept_map"),
        ),
        (
            r"\brevision[ -]?(pack|guide|sheet)\b|\bexam[ -]?(revision|review)\b|\bconsolidate\b|\bsynthesize\b",
            "M7",
            Some("revision_pack"),
        ),
        (
            r"^\s*(please\s+)?summari[sz]e\b|\bquick[ -]?summary\b|\bsummary of\b",
            "M7",
            Some("quick_summary"),
        ),
        (
            r"\b(study|course|lecture|chapter|syllabus)[ -]?(notes?|guide)\b",
            "M2",
            None,
        ),
        (r"\b(quick|brief|short)\b.{0,100}\breminder\b", "M3", None),
    ];
    for (pattern, master, variant) in rules {
        if forbidden.contains(master) || !matches(pattern, text) {
            continue;
        }
        if master == "M4"
            && matches(
                r"\bin (python|rust|java|c\+\+|cpp|go|javascript|typescript)\b",
                text,
            )
        {
            return ("M5".into(), None);
        }
        if master == "M2" && has_source {
            return (master.into(), Some("lecture_order".into()));
        }
        return (master.into(), variant.map(str::to_owned));
    }
    if has_source
        && matches(
            r"\b(make|create|write|take|convert|turn)\b.*\bnotes?\b",
            text,
        )
        && !forbidden.contains("M2")
    {
        return ("M2".into(), Some("lecture_order".into()));
    }
    if forbidden.contains("M1") {
        ("M2".into(), None)
    } else {
        ("M1".into(), None)
    }
}
pub fn extract_topic(request: &str) -> String {
    let text = regexp(r"^/?scribe-[\w-]+\s*")
        .replace(request, "")
        .to_string();
    let text = regexp(r"(?i)^\s*(please\s+)?(make|create|write|give me|generate)\s+")
        .replace(&text, "")
        .to_string();
    let text=regexp(r"(?i)^(comprehensive|complete|full|deep|detailed|concise|structured|quick|beginner|advanced)\s+").replace_all(&text,"").to_string();
    let text=regexp(r"(?i)^(notes?|study notes?|guide|cheat sheet|formula sheet|worked solutions|flashcards?|concept map)\s+(on|about|for|of)?\s*").replace(&text,"").to_string();
    regexp(r"(?i)\s+(using only|with |from this|in a single|in one file|multiple linked).*$")
        .replace(&text, "")
        .trim()
        .to_string()
}
pub fn route_request(request: &str, options: &RouteOptions) -> Result<RouteDecision> {
    ensure!(request.len() <= 100_000, "request too large");
    let request = request.trim();
    let first = request
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches('/');
    let public = [
        "scribe-notes",
        "scribe-doubt",
        "scribe-quiz",
        "scribe-setfolder",
    ];
    let retired = [
        "scribe-deepnotes",
        "scribe-distill",
        "scribe-compare",
        "scribe-dsa",
        "scribe-practice",
        "scribe-atomic-notes",
        "scribe-ingest",
        "scribe-expand",
    ];
    let retired_command = retired.contains(&first).then(|| first.to_string());
    let command = if public.contains(&first) {
        first
    } else {
        "scribe-notes"
    };
    let mode = match command {
        "scribe-doubt" => "query",
        "scribe-quiz" => "quiz",
        "scribe-setfolder" => "configuration",
        _ => "author",
    };
    let source_order = matches(
        r"\b(this|my|the)\s+(pdf|lecture|slides|chapter|transcript|source|paper)\b|\b(source|lecture|chapter)\s+order\b",
        request,
    );
    let strict = matches(
        r"\busing only\b|\bonly (from|using)\b|\bno outside\b|\bsource[- ]only\b|\bstrict(ly)? (source|faithful)\b|\bdo not add outside\b",
        request,
    );
    let policy = options.source_policy.clone().unwrap_or_else(|| {
        if strict {
            "strict"
        } else if options.has_source || source_order || matches(r"https?://", request) {
            "source_first"
        } else {
            "adaptive"
        }
        .into()
    });
    ensure!(
        ["strict", "source_first", "adaptive"].contains(&policy.as_str()),
        "unknown source policy"
    );
    let research = match policy.as_str() {
        "strict" => "R-S0",
        "source_first" => "R-S1",
        _ => "R-I",
    };
    let mut forbidden = BTreeSet::new();
    let mut negated = Vec::new();
    let cleaned=regexp(r"\b(don't|do not|dont|never|not|no|avoid|skip|without|instead of|rather than)\s+(make\s+|create\s+|a\s+|an\s+)*(cheat[ -]?sheet|formula[ -]?sheet|quick[ -]?reference|summary|flashcards?|quiz|practice[ -]?set|worked[ -]?solutions?|implementation|deep[ -]?notes?|study[ -]?notes?)\b").replace_all(request,|caps:&regex::Captures|{let phrase=caps.get(3).unwrap().as_str();let(m,_)=signals(phrase,false,&BTreeSet::new());forbidden.insert(m);negated.push(phrase.to_owned());" ".to_string()}).to_string();
    let mut secondary_modules = Vec::new();
    let cleaned_intent=regexp(r"\b(include|including|with|add|append)\s+(a\s+)?(short\s+|brief\s+|quick\s+|compact\s+|mini\s+)?(formula\s+)?(cheat[ -]?sheet|formula[ -]?sheet)(\s+at the end)?\b").replace_all(&cleaned,|_:&regex::Captures|{secondary_modules.push("formula_reference");" ".to_owned()}).to_string();
    let primary=regexp(r"\b(and|plus)\s+(also\s+)?(make\s+|create\s+)?(a\s+|an\s+)?(separate|standalone|companion|dedicated)\s+.*$").replace(&cleaned_intent,"").to_string();
    let (mut master, mut variant) =
        signals(&primary, options.has_source || source_order, &forbidden);
    let mut explicit = false;
    if let Some(c)=regexp(r#"\b(template|master)\s*[:=]\s*[`"']?(M[1-7]|deep-notes|study-notes|quick-reference|worked-solutions|implementation-guide|assessment|synthesis-revision)"#).captures(request){let token=c.get(2).unwrap().as_str();master=if token.starts_with('M')||token.starts_with('m'){token.to_uppercase()}else{match token.to_lowercase().as_str(){"study-notes"=>"M2","quick-reference"=>"M3","worked-solutions"=>"M4","implementation-guide"=>"M5","assessment"=>"M6","synthesis-revision"=>"M7",_=>"M1"}.into()};variant=None;explicit=true;}
    if request.starts_with("https://") && request.split_whitespace().count() == 1 {
        let url = url::Url::parse(request)?;
        master = "M7".into();
        variant = Some(
            if [
                "arxiv.org",
                "doi.org",
                "openreview.net",
                "aclanthology.org",
                "proceedings.neurips.cc",
            ]
            .contains(&url.host_str().unwrap_or(""))
            {
                "paper_breakdown"
            } else {
                "quick_summary"
            }
            .into(),
        );
    }
    if master == "M1" {
        match first {
            "scribe-distill" => master = "M3".into(),
            "scribe-compare" => {
                master = "M7".into();
                variant = Some("comparison_guide".into());
            }
            "scribe-dsa" => master = "M5".into(),
            "scribe-practice" => master = "M6".into(),
            "scribe-ingest" => {
                master = "M2".into();
                variant = Some("lecture_order".into());
            }
            _ => (),
        }
    }
    let artifact = |m: &str, v: Option<&str>| json!({"master":m,"variant":v,"compat_route":compatibility(m,v),"template_path":format!("references/templates/masters/{}.md",note_type(m)),"source_policy":policy,"research_mode":research,"reason":"native intent and explicit constraint evaluation"});
    let mut first_artifact = artifact(&master, variant.as_deref());
    first_artifact["secondary_modules"] = json!(secondary_modules);
    let mut artifacts = vec![first_artifact];
    if let Some(c) = regexp(r"\bboth\s+(.+?)\s+and\s+(.+)$").captures(&cleaned) {
        artifacts.clear();
        for index in [1, 2] {
            let (m, v) = signals(
                c.get(index).unwrap().as_str(),
                options.has_source,
                &forbidden,
            );
            artifacts.push(artifact(&m, v.as_deref()));
        }
        master = artifacts[0]["master"].as_str().unwrap().into();
        variant = artifacts[0]["variant"].as_str().map(str::to_owned);
    }
    for cap in
        regexp(r"\b(separate|standalone|companion|dedicated)\s+([^,;.!]+)").captures_iter(&cleaned)
    {
        let (m, v) = signals(cap.get(2).unwrap().as_str(), false, &forbidden);
        if !artifacts
            .iter()
            .any(|a| a["master"] == m && a["variant"] == json!(v))
        {
            artifacts.push(artifact(&m, v.as_deref()));
        }
    }
    let mut types = Vec::new();
    for (name, pattern) in [
        (
            "mathematical",
            "\\b(?:theorem|proof|lemma|calculus|integral|integration|derivative|linear algebra|matrix|matrices|eigen|bayes|probability|stochastic|differential equation|optimization|gradient descent|sgd|rmsprop|adam|momentum|optimizer|backpropagation|entropy|thermodynamics|lagrangian|fourier|laplace|formula|equation|biot[\\s–-]savart)\\b",
        ),
        (
            "computational",
            "\\b(?:python|numpy|pandas|pytorch|tensorflow|c\\+\\+|cpp|rust|java|javascript|typescript|sql|git|bash|cli|api|fastapi|rest|docker|kubernetes|algorithm|quicksort|mergesort|binary search|dijkstra|dynamic programming|binary tree|graph|transformer|neural network|cnn|vit|compiler|operating system|memory|cache|debug|implement|code|syntax)\\b",
        ),
        (
            "scientific",
            "\\b(?:physics|thermodynamics|electromagnetism|biot[\\s–-]savart|magnetostatics|quantum|mechanics|chemistry|organic chemistry|reaction|molecule|biology|mitosis|meiosis|cellular respiration|genetics|dna|enzyme|neuroscience|ecology|geology|astronomy|photosynthesis|evolution|optics)\\b",
        ),
        (
            "historical",
            "\\b(?:history|historical|revolution|french revolution|world war|treaty|treaties|empire|dynasty|renaissance|cold war|industrial revolution|timeline|civil war|ancient|medieval|colonial)\\b",
        ),
        (
            "empirical",
            "\\b(?:paper|arxiv|ablation|benchmark|experiment|dataset|empirical|clinical trial|meta-analysis|p-value|regression study|survey data|evaluation metric|sota|baseline)\\b",
        ),
        (
            "conceptual",
            "\\b(?:economics|keynesian|monetarist|microeconomics|macroeconomics|philosophy|ethics|epistemology|law|constitutional|sociology|psychology|cognitive|framework|architecture|design pattern|management|strategy|governance|theory)\\b",
        ),
    ] {
        if matches(pattern, request) {
            types.push(name.to_owned());
        }
    }
    if types.is_empty() {
        types.push("conceptual".into());
    }
    let multi = artifacts.len() > 1;
    Ok(RouteDecision {
        command: command.into(),
        mode: mode.into(),
        compat_route: compatibility(&master, variant.as_deref()),
        lock: (explicit || master != "M1").then(|| "explicit".into()),
        master,
        variant,
        source_policy: policy,
        research_mode: research.into(),
        multi_artifact: multi,
        shared_context: multi,
        artifacts,
        knowledge_types: types,
        routing_trace: json!({"source_order_preserved":source_order,"negated_artifacts":negated,"explicit_template":explicit,"artifact_count":if multi{2}else{1}}),
        retired_command: retired_command.clone(),
        migration_hint: retired_command
            .map(|c| format!("/{c} has been consolidated into /scribe-notes")),
        topic: extract_topic(request),
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScopeDecision {
    pub level: String,
    pub recommended_structure: String,
    pub requires_scope_boundary: bool,
    pub rationale: String,
}
pub fn normalize_request(request: &str, options: &RouteOptions) -> Result<Value> {
    let routed = route_request(request, options)?;
    let mathematical = if matches(
        r"\b(all the math|full derivations?|proof|rigorous math|all equations|all formulas)\b",
        request,
    ) {
        "deep"
    } else if matches(r"\b(equations?|formulas?|math|derivations?)\b", request) {
        "detailed"
    } else {
        "standard"
    };
    let conceptual = if matches(
        r"\b(very detailed|comprehensive|deep(ly)?|first[ -]?principles|entire|exhaustive)\b",
        request,
    ) {
        "deep"
    } else {
        "standard"
    };
    let coding = if matches(r"\b(without|no|skip|avoid) (code|coding)\b", request) {
        "none"
    } else if matches(
        r"\b(python|numpy|pytorch|rust|cpp|code|implement|script|fastapi)\b",
        request,
    ) {
        "detailed"
    } else {
        "minimal"
    };
    let audience = if matches(
        r"\b(beginner|introductory|from scratch|simple|eli5)\b",
        request,
    ) {
        "beginner"
    } else if matches(
        r"\b(advanced|graduate|research[- ]level|rigorous)\b",
        request,
    ) {
        "advanced"
    } else {
        "intermediate"
    };
    let mut required = Vec::new();
    for (p, label) in [
        (r"\b(worked\s+)?examples?\b", "worked_examples"),
        (
            r"\b(computational graph|diagram|visual)\b",
            "visual_diagrams",
        ),
        (
            r"\b(python|numpy|pytorch|code|implementation)\b",
            "code_implementation",
        ),
        (
            r"\b(all (the )?(formulas?|equations?)|every formula)\b",
            "all_source_formulas",
        ),
        (r"\b(derivations?|proofs?)\b", "derivations"),
        (r"\b(fast|quick) revision\b", "revision_summary"),
    ] {
        if matches(p, request) {
            required.push(label);
        }
    }
    let has_source = options.has_source || matches(r"https?://", request);
    let mut ambiguity = Vec::new();
    if matches(
        r"\b(this|the) (lecture|pdf|chapter|paper|slides)\b",
        request,
    ) && !has_source
    {
        ambiguity.push("missing_referenced_source");
    }
    let outputs = routed
        .artifacts
        .iter()
        .filter_map(|a| a["master"].as_str())
        .map(note_type)
        .collect::<Vec<_>>();
    Ok(
        json!({"topic":routed.topic,"primary_goal":match routed.master.as_str(){"M2"=>"structured_study","M3"=>"rapid_reference","M4"=>"worked_problem_solving","M5"=>"practical_implementation","M6"=>"knowledge_assessment","M7"=>"synthesis_and_revision",_=>"comprehensive_learning"},"command":routed.command,"requested_outputs":outputs,"artifacts":routed.artifacts,"artifact_relationship":{"separate_files":routed.multi_artifact,"shared_context":routed.shared_context,"artifact_count":routed.artifacts.len()},"depth":{"conceptual":conceptual,"mathematical":mathematical,"coding":coding},"audience":audience,"knowledge_types":routed.knowledge_types,"constraints":{"source_policy":routed.source_policy,"research_mode":routed.research_mode,"preserve_source_order":routed.routing_trace["source_order_preserved"],"has_source":has_source},"required_elements":required,"excluded_elements":routed.routing_trace["negated_artifacts"],"secondary_modules":routed.artifacts[0]["secondary_modules"],"ambiguity":ambiguity,"routed":routed}),
    )
}
pub fn resolve_constraints(normalized: &Value, extra: &[Value]) -> Result<Value> {
    let rank = [
        "runtime_safety",
        "explicit_user",
        "source_policy",
        "command_contract",
        "primary_master",
        "variant_specialization",
        "knowledge_type",
        "optional_module",
        "default_preference",
    ];
    let mut conflicts = Vec::new();
    let mut adaptations = Vec::new();
    let required = normalized["required_elements"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let policy = normalized["constraints"]["source_policy"]
        .as_str()
        .unwrap_or("adaptive");
    if policy == "strict" && required.contains(&json!("derivations")) {
        conflicts.push(json!({"prevailing_layer":"source_policy","reason":"Restrict derivations to source premises and steps","adapted":true,"satisfies_user_objective":true}));
        adaptations.push("restrict_derivations_to_source_premises");
    }
    if normalized["artifacts"][0]["master"] == "M3"
        && required.contains(&json!("all_source_formulas"))
    {
        conflicts.push(json!({"prevailing_layer":"explicit_user","reason":"Preserve every requested source formula with compact prose","adapted":true,"satisfies_user_objective":true}));
        adaptations.push("preserve_all_source_formulas_with_compact_prose");
    }
    if normalized["excluded_elements"].as_array().is_some_and(|a| {
        a.iter()
            .any(|e| e.as_str().is_some_and(|s| s.contains("example")))
    }) {
        adaptations.push("omit_worked_examples_per_user_exclusion");
    }
    for rule in extra {
        let a = rule["layer_a"].as_str().unwrap_or("primary_master");
        let b = rule["layer_b"].as_str().unwrap_or("optional_module");
        let ra = rank
            .iter()
            .position(|r| *r == a)
            .ok_or_else(|| anyhow::anyhow!("unknown constraint layer {a}"))?;
        let rb = rank
            .iter()
            .position(|r| *r == b)
            .ok_or_else(|| anyhow::anyhow!("unknown constraint layer {b}"))?;
        conflicts.push(json!({"conflicting_requirements":[format!("{a}: {}",rule["rule_a"]),format!("{b}: {}",rule["rule_b"])],"prevailing_layer":if ra<=rb{a}else{b},"reason":"Instruction layer precedence","adapted":true,"satisfies_user_objective":true}));
    }
    Ok(
        json!({"precedence_order":rank,"primary_master":normalized["artifacts"][0]["master"],"primary_variant":normalized["artifacts"][0]["variant"],"effective_source_policy":policy,"conflicts":conflicts,"adaptations":adaptations,"resolved":true}),
    )
}
pub fn evaluate_clarification(normalized: &Value, destructive: bool, authorized: bool) -> Value {
    let mut questions = Vec::new();
    if destructive && !authorized {
        questions.push("Confirm the irreversible operation before proceeding.");
    }
    if normalized["ambiguity"]
        .as_array()
        .is_some_and(|a| a.contains(&json!("missing_referenced_source")))
    {
        questions.push("Supply the referenced lecture/PDF/chapter file or URL.");
    }
    if normalized["constraints"]["source_policy"] == "strict"
        && normalized["required_elements"]
            .as_array()
            .is_some_and(|a| a.contains(&json!("external_research")))
    {
        questions.push("Choose whether source-only fidelity or external research should prevail.");
    }
    json!({"action":if questions.is_empty(){"proceed"}else{"ask"},"questions":questions,"assumptions":[{"id":"default_master_selection","value":normalized["requested_outputs"][0],"basis":"safe_default"},{"id":"audience_baseline","value":normalized["audience"],"basis":"inferred"}]})
}
pub fn classify_scope(topic: &str, request: &str) -> ScopeDecision {
    let extensive = matches(
        r"\b(entire field|all of|whole field|complete knowledge base|every aspect)\b",
        request,
    );
    let broad = [
        "computer science",
        "artificial intelligence",
        "mathematics",
        "physics",
        "biology",
        "chemistry",
        "engineering",
    ]
    .contains(&topic.to_lowercase().as_str());
    let major = [
        "machine learning",
        "deep learning",
        "operating systems",
        "linear algebra",
        "calculus",
        "thermodynamics",
        "electromagnetism",
    ]
    .iter()
    .any(|t| topic.eq_ignore_ascii_case(t));
    let level = if topic.trim().is_empty() {
        "unknown"
    } else if extensive {
        "S5"
    } else if broad {
        "S4"
    } else if major {
        "S3"
    } else if matches(r"\b(optimizers|algorithms|architectures|methods)\b", topic) {
        "S2"
    } else if matches(r"\b(atomic|single definition|one formula)\b", request) {
        "S0"
    } else {
        "S1"
    };
    ScopeDecision {
        level: level.into(),
        recommended_structure: if ["S3", "S4", "S5"].contains(&level) {
            "hierarchical_collection"
        } else {
            "single_note"
        }
        .into(),
        requires_scope_boundary: ["S4", "S5"].contains(&level),
        rationale: format!(
            "Request classified as {level}; explicit organization and source boundaries take precedence"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_all_masters_and_variants() {
        for (r, m, v) in [
            ("Gradient Descent", "M1", None),
            (
                "Structured study notes on Biot-Savart from this lecture",
                "M2",
                Some("lecture_order"),
            ),
            (
                "Formula sheet for Maxwell equations",
                "M3",
                Some("formula_sheet"),
            ),
            (
                "Solve these 5 integration problems step by step",
                "M4",
                None,
            ),
            ("Implement binary search in Python", "M5", None),
            (
                "Create flashcards for respiration",
                "M6",
                Some("flashcards"),
            ),
            (
                "Compare CNN vs Vision Transformers",
                "M7",
                Some("comparison_guide"),
            ),
        ] {
            let d = route_request(r, &RouteOptions::default()).unwrap();
            assert_eq!(d.master, m, "{r}");
            assert_eq!(d.variant.as_deref(), v, "{r}");
        }
    }
    #[test]
    fn preserves_source_and_explicit_constraints() {
        let d = route_request(
            "deep notes using only my source, master: M3",
            &RouteOptions::default(),
        )
        .unwrap();
        assert_eq!(d.master, "M3");
        assert_eq!(d.source_policy, "strict");
        assert_eq!(d.research_mode, "R-S0");
        let d = route_request(
            "Do not make a cheat sheet, explain gradients",
            &RouteOptions::default(),
        )
        .unwrap();
        assert_eq!(d.master, "M1");
    }

    #[test]
    fn original_canonical_routing_fixtures() {
        for (request, master, variant) in [
            (
                "Explain transformers deeply from first principles",
                "M1",
                None,
            ),
            (
                "Make notes on this lecture without skipping anything",
                "M2",
                Some("lecture_order"),
            ),
            ("Give me a cheat sheet for linear algebra", "M3", None),
            (
                "Make a formula sheet for thermodynamics",
                "M3",
                Some("formula_sheet"),
            ),
            (
                "Give me a PyTorch tensor syntax reference",
                "M3",
                Some("syntax_reference"),
            ),
            (
                "Give me a Git command reference",
                "M3",
                Some("command_reference"),
            ),
            (
                "Create an atomic note for Vanishing Gradient",
                "M3",
                Some("atomic_card"),
            ),
            (
                "Solve these 10 calculus integration problems step by step",
                "M4",
                None,
            ),
            ("Show worked examples of Bayes theorem", "M4", None),
            (
                "Implement gradient descent from scratch in NumPy",
                "M5",
                None,
            ),
            (
                "Build a REST API in FastAPI and debug authentication",
                "M5",
                None,
            ),
            ("Make a practice test on neural networks", "M6", None),
            (
                "Create flashcards for cellular respiration",
                "M6",
                Some("flashcards"),
            ),
            (
                "Create a mock exam on operating systems",
                "M6",
                Some("mock_exam"),
            ),
            ("Compare CNN vs ViT", "M7", Some("comparison_guide")),
            (
                "Create an exam revision pack from my 5 calculus notes",
                "M7",
                Some("revision_pack"),
            ),
            (
                "Make a concept map of machine learning algorithms",
                "M7",
                Some("concept_map"),
            ),
            (
                "Break down this paper https://arxiv.org/abs/1706.03762",
                "M7",
                Some("paper_breakdown"),
            ),
            (
                "Summarize this article https://example.org/attention-guide",
                "M7",
                Some("quick_summary"),
            ),
            (
                "Make a cheat sheet for organic chemistry reactions",
                "M3",
                None,
            ),
            (
                "Create a formula sheet for microeconomics elasticities",
                "M3",
                Some("formula_sheet"),
            ),
            (
                "Solve these 5 organic chemistry synthesis problems step by step",
                "M4",
                None,
            ),
            (
                "Make flashcards for World War II treaties",
                "M6",
                Some("flashcards"),
            ),
            ("Compare Mitosis vs Meiosis", "M7", Some("comparison_guide")),
            (
                "Explain the French Revolution from first principles",
                "M1",
                None,
            ),
            (
                "Compare Keynesian vs Monetarist economics",
                "M7",
                Some("comparison_guide"),
            ),
            ("/scribe-notes Gradient Descent", "M1", None),
            (
                "/scribe-notes Structured study notes on Biot-Savart Law from this lecture",
                "M2",
                Some("lecture_order"),
            ),
            (
                "/scribe-notes Make a cheat sheet for Pandas Data Cleaning",
                "M3",
                None,
            ),
            (
                "/scribe-notes Formula sheet for Maxwell's Equations",
                "M3",
                Some("formula_sheet"),
            ),
            (
                "/scribe-notes Atomic note on Residual Connection",
                "M3",
                Some("atomic_card"),
            ),
            (
                "/scribe-notes Solve these 5 calculus integration problems step by step",
                "M4",
                None,
            ),
            (
                "/scribe-notes Implement binary search from scratch in Python",
                "M5",
                None,
            ),
            (
                "/scribe-notes Create flashcards for cellular respiration",
                "M6",
                Some("flashcards"),
            ),
            (
                "/scribe-notes Compare CNN vs Vision Transformers",
                "M7",
                Some("comparison_guide"),
            ),
            (
                "/scribe-notes Concept map of Thermodynamics",
                "M7",
                Some("concept_map"),
            ),
        ] {
            let result = route_request(request, &RouteOptions::default()).unwrap();
            assert_eq!(result.master, master, "{request}");
            assert_eq!(result.variant.as_deref(), variant, "{request}");
        }
    }
}
