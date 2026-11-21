//! Obsidian properties, shared aliases and hierarchical taxonomy; no interpreter required.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn slugify(value: &str) -> String {
    let mut out = String::new();
    for c in value.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let s = out.trim_matches('-');
    if s.is_empty() {
        "note".into()
    } else {
        s.into()
    }
}
pub fn canonical_note_type(route: &str) -> &'static str {
    match route.trim().to_lowercase().as_str() {
        "study-notes" | "studynotes" => "study-notes",
        "quick-reference" | "cheat-sheet" | "api-table" | "atomic" => "quick-reference",
        "worked-solutions" => "worked-solutions",
        "implementation-guide" | "dsa" => "implementation-guide",
        "assessment" | "practice" => "assessment",
        "synthesis-revision" | "comparison" | "summary" | "paper-breakdown" => "synthesis-revision",
        "index" => "index",
        _ => "deep-notes",
    }
}
pub fn normalize_tag_segment(raw: &str) -> String {
    let slug = slugify(raw.trim().trim_start_matches('#'));
    match slug.as_str() {
        "ml" | "machinelearning" => "machine-learning".into(),
        "ai" | "artificialintelligence" => "artificial-intelligence".into(),
        "dl" | "deeplearning" => "deep-learning".into(),
        "dsa" | "data-structures-and-algorithms" => "data-structures-algorithms".into(),
        "cs" => "computer-science".into(),
        "os" => "operating-systems".into(),
        "nlp" => "natural-language-processing".into(),
        "cv" => "computer-vision".into(),
        "gd" => "gradient-descent".into(),
        "sgd" => "stochastic-gradient-descent".into(),
        "py" | "python3" => "python".into(),
        "cpp" => "c-plus-plus".into(),
        "js" => "javascript".into(),
        "ts" => "typescript".into(),
        "em" => "electromagnetism".into(),
        "ww2" | "wwii" => "world-war-ii".into(),
        _ if raw.trim().eq_ignore_ascii_case("c++") => "c-plus-plus".into(),
        _ => slug,
    }
}
pub fn normalize_tag(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('#')
        .split('/')
        .filter(|p| !p.trim().is_empty())
        .map(normalize_tag_segment)
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hierarchy {
    pub domain: String,
    pub master_topic: String,
    pub subdomain: String,
}
pub fn classify(topic: &str, content: &str) -> Hierarchy {
    let find = |input: &str| -> Option<Hierarchy> {
        for (pattern, domain, master, sub) in TAXONOMY {
            if crate::orchestration::matches(pattern, input) {
                return Some(Hierarchy {
                    domain: (*domain).into(),
                    master_topic: (*master).into(),
                    subdomain: (*sub).into(),
                });
            }
        }
        None
    };
    let mut h = find(topic)
        .or_else(|| find(&content.chars().take(600).collect::<String>()))
        .unwrap_or(Hierarchy {
            domain: "General".into(),
            master_topic: "General Knowledge".into(),
            subdomain: String::new(),
        });
    if h.domain == "Physics"
        && h.master_topic == "Electromagnetism"
        && crate::orchestration::matches(
            r"\b(gauss|coulomb|electrostatic|electric\s+field)\b",
            topic,
        )
    {
        h.subdomain = "Electrostatics".into();
    }
    if h.domain == "History"
        && crate::orchestration::matches(r"\b(roman|greek|ancient|mesopotamia)\b", topic)
    {
        h.subdomain = "Ancient History".into();
    }
    h
}
pub fn generate_tags(topic: &str, subject: &str, domain: &str) -> Vec<String> {
    [
        format!("topic/{topic}"),
        format!("subject/{subject}"),
        format!("domain/{domain}"),
    ]
    .iter()
    .map(|s| normalize_tag(s))
    .collect()
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PropertiesOptions {
    pub domain: Option<String>,
    pub master_topic: Option<String>,
    pub subdomain: Option<String>,
    pub content: String,
    pub secondary_concepts: Vec<String>,
    pub user_tags: Vec<String>,
    pub custom_properties: BTreeMap<String, Value>,
    pub id: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub nexttopics: Option<Vec<String>>,
    pub parent_topic: Option<String>,
    pub collection_id: Option<String>,
    pub difficulty: Option<String>,
    #[serde(flatten)]
    pub optional: BTreeMap<String, Value>,
}
pub fn properties(topic: &str, note_type: &str, options: &PropertiesOptions) -> Result<Value> {
    ensure!(
        !topic.trim().is_empty(),
        "Obsidian Properties require a nonempty topic"
    );
    let mut hierarchy = classify(topic, &options.content);
    if let Some(v) = &options.domain
        && !v.trim().is_empty()
        && !v.eq_ignore_ascii_case("general")
    {
        hierarchy.domain = v.trim().into();
    }
    if let Some(v) = &options.master_topic
        && !v.trim().is_empty()
        && !v.eq_ignore_ascii_case("general")
    {
        hierarchy.master_topic = v.trim().into();
    }
    if let Some(v) = &options.subdomain {
        hierarchy.subdomain = v.trim().into();
    }
    let kind = canonical_note_type(note_type);
    let mut tags = generate_tags(topic, &hierarchy.master_topic, &hierarchy.domain);
    for tag in [
        format!("master-topic/{}", hierarchy.master_topic),
        format!("concept/{topic}"),
    ] {
        tags.push(normalize_tag(&tag));
    }
    if !hierarchy.subdomain.is_empty() {
        tags.insert(
            4,
            normalize_tag(&format!("subdomain/{}", hierarchy.subdomain)),
        );
    }
    for concept in options.secondary_concepts.iter().take(3) {
        let tag = normalize_tag(&format!("concept/{concept}"));
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags.push(format!("type/{kind}"));
    if let Some(d) = &options.difficulty
        && ["beginner", "intermediate", "advanced"].contains(&d.as_str())
    {
        tags.push(format!("level/{d}"));
    }
    for tag in &options.user_tags {
        let raw = tag.trim().trim_start_matches('#');
        if !raw.is_empty() && !tags.contains(&raw.to_owned()) && !tags.contains(&normalize_tag(raw))
        {
            tags.push(raw.to_owned());
        }
    }
    let mut p = options.custom_properties.clone();
    for (key, value) in &options.optional {
        ensure!(
            !["topic", "tags", "master_topic", "domain", "note_type"].contains(&key.as_str()),
            "reserved Properties override"
        );
        p.insert(key.clone(), value.clone());
    }
    for (key, value) in [
        ("topic", json!(topic.trim())),
        ("tags", json!(tags)),
        ("master_topic", json!(hierarchy.master_topic)),
        ("domain", json!(hierarchy.domain)),
        ("note_type", json!(kind)),
    ] {
        p.insert(key.into(), value);
    }
    if !hierarchy.subdomain.is_empty() {
        p.insert("subdomain".into(), json!(hierarchy.subdomain));
    }
    for (k, v) in [
        ("id", &options.id),
        ("parent_topic", &options.parent_topic),
        ("collection_id", &options.collection_id),
        ("difficulty", &options.difficulty),
    ] {
        if let Some(s) = v {
            p.insert(k.into(), json!(s));
        }
    }
    if let Some(v) = &options.aliases {
        p.insert("aliases".into(), json!(v));
    }
    if let Some(v) = &options.nexttopics {
        p.insert("nexttopics".into(), json!(v));
    }
    Ok(json!(p))
}
pub fn build_properties(
    topic: &str,
    note_type: &str,
    domain: Option<&str>,
    master_topic: Option<&str>,
) -> Result<Value> {
    properties(
        topic,
        note_type,
        &PropertiesOptions {
            domain: domain.map(str::to_owned),
            master_topic: master_topic.map(str::to_owned),
            ..Default::default()
        },
    )
}
pub fn render_note(properties: &Value, body: &str) -> Result<String> {
    ensure!(properties.is_object(), "Properties must be mapping");
    let yaml = serde_yaml_ng::to_string(properties)?;
    let text = format!("---\n{yaml}---\n{}\n", body.trim());
    crate::validation::validate_properties(&text)?;
    Ok(text)
}

const TAXONOMY: &[(&str, &str, &str, &str)] = &[
    (
        "\\b(?:circuits?|electrical\\s+engineering|thevenin|thévenin|kirchhoff|impedance|electronics)\\b",
        "Engineering",
        "Electrical Engineering",
        "Circuit Analysis",
    ),
    (
        "\\b(?:control\\s+systems?|feedback\\s+control|transfer\\s+functions?)\\b",
        "Engineering",
        "Control Engineering",
        "Systems",
    ),
    (
        "\\b(?:gradient\\s+descent|stochastic\\s+gradient|adam\\s+optimizer|rmsprop|momentum|learning\\s+rate|convex\\s+optimization)\\b",
        "Artificial Intelligence",
        "Machine Learning",
        "Optimization",
    ),
    (
        "\\b(?:backpropagation|transformer|self[- ]attention|attention\\s+mechanism|neural\\s+network|cnn|convolutional|vit|vision\\s+transformer|rnn|lstm|residual\\s+connection|vanishing\\s+gradient|batch\\s+norm|layer\\s+norm|deep\\s+learning|mlp|perceptron)\\b",
        "Artificial Intelligence",
        "Machine Learning",
        "Deep Learning",
    ),
    (
        "\\b(?:machine\\s+learning|supervised\\s+learning|unsupervised|regression|classification|svm|random\\s+forest|decision\\s+tree|k[- ]means|pca|loss\\s+function|overfitting|regularization|cross[- ]validation|scikit[- ]learn)\\b",
        "Artificial Intelligence",
        "Machine Learning",
        "Foundations",
    ),
    (
        "\\b(?:pandas|polars|data\\s+cleaning|dataframe|numpy|scipy|matplotlib)\\b",
        "Computer Science",
        "Python Programming",
        "Data Processing",
    ),
    (
        "\\b(?:pytorch|tensorflow|keras)\\b",
        "Artificial Intelligence",
        "Machine Learning",
        "Frameworks",
    ),
    (
        "\\b(?:binary\\s+search|linear\\s+search)\\b",
        "Computer Science",
        "Data Structures and Algorithms",
        "Searching",
    ),
    (
        "\\b(?:quicksort|mergesort|heapsort|sorting\\s+algorithm)\\b",
        "Computer Science",
        "Data Structures and Algorithms",
        "Sorting",
    ),
    (
        "\\b(?:two\\s+sum|dynamic\\s+programming|dijkstra|bfs|dfs|binary\\s+tree|red[- ]black\\s+tree|trie|hash\\s+table|linked\\s+list|graph\\s+algorithm|dsa|data\\s+structures?|algorithms?|recursion|heap)\\b",
        "Computer Science",
        "Data Structures and Algorithms",
        "Algorithms",
    ),
    (
        "\\b(?:operating\\s+systems?|paging|virtual\\s+memory|cpu\\s+scheduling|deadlock|threads?|mutex|semaphore|process\\s+scheduling|linux\\s+kernel)\\b",
        "Computer Science",
        "Operating Systems",
        "Systems",
    ),
    (
        "\\b(?:fastapi|rest\\s+api|graphql|http|websocket|authentication|oauth|jwt|react|typescript|javascript|frontend|backend)\\b",
        "Computer Science",
        "Software Engineering",
        "Web Development",
    ),
    (
        "\\b(?:git\\b|docker|kubernetes|bash|shell|cli|linux\\s+commands?|ci/cd)\\b",
        "Computer Science",
        "Software Engineering",
        "Developer Tools",
    ),
    (
        "\\b(?:sql|postgresql|mysql|duckdb|database|b[- ]tree|acid|transaction\\s+isolation|indexing|normalization\\s+forms?|3nf)\\b",
        "Computer Science",
        "Databases",
        "Relational Systems",
    ),
    (
        "\\b(?:python|c\\+\\+|rust|java|golang|compiler|programming)\\b",
        "Computer Science",
        "Software Engineering",
        "Programming",
    ),
    (
        "\\b(?:biot[\\s–-]savart|ampere|magnetostatics|magnetic\\s+field|lorentz\\s+force|faraday|maxwell|gauss(?:'s)?\\s+law|electrostatics|electromagnetism|coulomb)\\b",
        "Physics",
        "Electromagnetism",
        "Magnetostatics",
    ),
    (
        "\\b(?:thermodynamics|entropy|carnot|enthalpy|gibbs\\s+free\\s+energy|boltzmann|heat\\s+engine|ideal\\s+gas)\\b",
        "Physics",
        "Thermodynamics",
        "Statistical Physics",
    ),
    (
        "\\b(?:wave\\s+optics|huygens|snell|refraction|diffraction|interference|polarization|lenses?|mirrors?|optics)\\b",
        "Physics",
        "Optics",
        "Wave Optics",
    ),
    (
        "\\b(?:quantum\\s+mechanics|schr[oö]dinger|wavefunction|heisenberg|superposition|entanglement|compton|photoelectric)\\b",
        "Physics",
        "Quantum Mechanics",
        "Foundations",
    ),
    (
        "\\b(?:kinematics|newton(?:'s)?\\s+laws?|lagrangian|hamiltonian|classical\\s+mechanics|relativity|physics)\\b",
        "Physics",
        "Classical Mechanics",
        "Dynamics",
    ),
    (
        "\\b(?:calculus|integrals?|integration|derivatives?|chain\\s+rule|taylor\\s+series|limits?|multivariable|partial\\s+derivatives?|differential\\s+equations?)\\b",
        "Mathematics",
        "Calculus",
        "Analysis",
    ),
    (
        "\\b(?:linear\\s+algebra|matrix|matrices|eigenvalue|eigenvector|svd|singular\\s+value|vector\\s+space|determinant|orthogonality)\\b",
        "Mathematics",
        "Linear Algebra",
        "Matrix Theory",
    ),
    (
        "\\b(?:bayes(?:'s?)?\\s+theorem|probability|random\\s+variable|markov\\s+chain|gaussian|normal\\s+distribution|central\\s+limit|hypothesis\\s+testing|statistics|stochastic)\\b",
        "Mathematics",
        "Probability and Statistics",
        "Inference",
    ),
    (
        "\\b(?:real\\s+analysis|topology|group\\s+theory|number\\s+theory|abstract\\s+algebra|measure\\s+theory|fourier|laplace|pure\\s+math)\\b",
        "Mathematics",
        "Pure Mathematics",
        "Analysis",
    ),
    (
        "\\b(?:organic\\s+chemistry|alkane|alkene|sn1|sn2|stereochemistry|carbonyl|aromatic|synthesis\\s+reaction)\\b",
        "Chemistry",
        "Organic Chemistry",
        "Reactions",
    ),
    (
        "\\b(?:chemistry|stoichiometry|chemical\\s+equilibrium|kinetics|acid[- ]base|electrochemistry|orbitals?|molecular\\s+bonding)\\b",
        "Chemistry",
        "General Chemistry",
        "Principles",
    ),
    (
        "\\b(?:cellular\\s+respiration|glycolysis|krebs\\s+cycle|atp|mitosis|meiosis|photosynthesis|organelle|cell\\s+cycle)\\b",
        "Biology",
        "Cell Biology",
        "Metabolism and Division",
    ),
    (
        "\\b(?:dna|rna|genetics|transcription|translation|crispr|enzyme|hardy[\\s–-]weinberg|evolution|ecology|biology)\\b",
        "Biology",
        "Molecular Biology and Genetics",
        "Genetics",
    ),
    (
        "\\b(?:french\\s+revolution|industrial\\s+revolution|world\\s+war|treaty|treaties|cold\\s+war|renaissance|roman\\s+empire|ottoman|dynasty|colonial|civil\\s+war|history|historical)\\b",
        "History",
        "World History",
        "Modern History",
    ),
    (
        "\\b(?:microeconomics|supply\\s+and\\s+demand|elasticity|marginal\\s+utility|monopoly|game\\s+theory|nash\\s+equilibrium)\\b",
        "Economics",
        "Microeconomics",
        "Market Theory",
    ),
    (
        "\\b(?:keynesian|monetarist|macroeconomics|inflation|monetary\\s+policy|fiscal\\s+policy|gdp|economics)\\b",
        "Economics",
        "Macroeconomics",
        "Economic Policy",
    ),
    (
        "\\b(?:philosophy|epistemology|ethics|kantian|utilitarianism|logic|metaphysics)\\b",
        "Philosophy",
        "Philosophy",
        "Foundations",
    ),
    (
        "\\b(?:constitutional\\s+law|contract\\s+law|tort|jurisprudence|statute)\\b",
        "Law",
        "Legal Studies",
        "Jurisprudence",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_tags_and_properties() {
        assert_eq!(
            normalize_tag("#domain/AI"),
            "domain/artificial-intelligence"
        );
        assert_eq!(normalize_tag_segment("C++"), "c-plus-plus");
        let p = build_properties("Binary Search", "dsa", None, None).unwrap();
        assert_eq!(p["domain"], "Computer Science");
        assert_eq!(p["note_type"], "implementation-guide");
        render_note(&p, "# Binary Search").unwrap();
    }
    #[test]
    fn preserves_custom_values_and_overrides() {
        let p = properties(
            "Unfamiliar topic",
            "deep-notes",
            &PropertiesOptions {
                domain: Some("History".into()),
                master_topic: Some("Manuscripts".into()),
                custom_properties: BTreeMap::from([("custom".into(), json!([1, 2]))]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p["custom"], json!([1, 2]));
        assert_eq!(p["domain"], "History");
    }

    #[test]
    fn original_metadata_fixture_parity() {
        for (topic, expected) in [
            (
                "Gradient Descent",
                r#"{"topic": "Gradient Descent", "tags": ["topic/gradient-descent", "subject/machine-learning", "domain/artificial-intelligence", "master-topic/machine-learning", "subdomain/optimization", "concept/gradient-descent", "type/deep-notes"], "master_topic": "Machine Learning", "domain": "Artificial Intelligence", "note_type": "deep-notes", "subdomain": "Optimization"}"#,
            ),
            (
                "Transformer Architecture",
                r#"{"topic": "Transformer Architecture", "tags": ["topic/transformer-architecture", "subject/machine-learning", "domain/artificial-intelligence", "master-topic/machine-learning", "subdomain/deep-learning", "concept/transformer-architecture", "type/deep-notes"], "master_topic": "Machine Learning", "domain": "Artificial Intelligence", "note_type": "deep-notes", "subdomain": "Deep Learning"}"#,
            ),
            (
                "Binary Search",
                r#"{"topic": "Binary Search", "tags": ["topic/binary-search", "subject/data-structures-algorithms", "domain/computer-science", "master-topic/data-structures-algorithms", "subdomain/searching", "concept/binary-search", "type/deep-notes"], "master_topic": "Data Structures and Algorithms", "domain": "Computer Science", "note_type": "deep-notes", "subdomain": "Searching"}"#,
            ),
            (
                "French Revolution",
                r#"{"topic": "French Revolution", "tags": ["topic/french-revolution", "subject/world-history", "domain/history", "master-topic/world-history", "subdomain/modern-history", "concept/french-revolution", "type/deep-notes"], "master_topic": "World History", "domain": "History", "note_type": "deep-notes", "subdomain": "Modern History"}"#,
            ),
            (
                "Cellular Respiration",
                r#"{"topic": "Cellular Respiration", "tags": ["topic/cellular-respiration", "subject/cell-biology", "domain/biology", "master-topic/cell-biology", "subdomain/metabolism-and-division", "concept/cellular-respiration", "type/deep-notes"], "master_topic": "Cell Biology", "domain": "Biology", "note_type": "deep-notes", "subdomain": "Metabolism and Division"}"#,
            ),
            (
                "Microeconomics",
                r#"{"topic": "Microeconomics", "tags": ["topic/microeconomics", "subject/microeconomics", "domain/economics", "master-topic/microeconomics", "subdomain/market-theory", "concept/microeconomics", "type/deep-notes"], "master_topic": "Microeconomics", "domain": "Economics", "note_type": "deep-notes", "subdomain": "Market Theory"}"#,
            ),
            (
                "Calculus",
                r#"{"topic": "Calculus", "tags": ["topic/calculus", "subject/calculus", "domain/mathematics", "master-topic/calculus", "subdomain/analysis", "concept/calculus", "type/deep-notes"], "master_topic": "Calculus", "domain": "Mathematics", "note_type": "deep-notes", "subdomain": "Analysis"}"#,
            ),
            (
                "Biot-Savart Law",
                r#"{"topic": "Biot-Savart Law", "tags": ["topic/biot-savart-law", "subject/electromagnetism", "domain/physics", "master-topic/electromagnetism", "subdomain/magnetostatics", "concept/biot-savart-law", "type/deep-notes"], "master_topic": "Electromagnetism", "domain": "Physics", "note_type": "deep-notes", "subdomain": "Magnetostatics"}"#,
            ),
            (
                "Pandas Data Cleaning",
                r#"{"topic": "Pandas Data Cleaning", "tags": ["topic/pandas-data-cleaning", "subject/python-programming", "domain/computer-science", "master-topic/python-programming", "subdomain/data-processing", "concept/pandas-data-cleaning", "type/deep-notes"], "master_topic": "Python Programming", "domain": "Computer Science", "note_type": "deep-notes", "subdomain": "Data Processing"}"#,
            ),
            (
                "FastAPI",
                r#"{"topic": "FastAPI", "tags": ["topic/fastapi", "subject/software-engineering", "domain/computer-science", "master-topic/software-engineering", "subdomain/web-development", "concept/fastapi", "type/deep-notes"], "master_topic": "Software Engineering", "domain": "Computer Science", "note_type": "deep-notes", "subdomain": "Web Development"}"#,
            ),
            (
                "Organic Chemistry",
                r#"{"topic": "Organic Chemistry", "tags": ["topic/organic-chemistry", "subject/organic-chemistry", "domain/chemistry", "master-topic/organic-chemistry", "subdomain/reactions", "concept/organic-chemistry", "type/deep-notes"], "master_topic": "Organic Chemistry", "domain": "Chemistry", "note_type": "deep-notes", "subdomain": "Reactions"}"#,
            ),
            (
                "Thermodynamics",
                r#"{"topic": "Thermodynamics", "tags": ["topic/thermodynamics", "subject/thermodynamics", "domain/physics", "master-topic/thermodynamics", "subdomain/statistical-physics", "concept/thermodynamics", "type/deep-notes"], "master_topic": "Thermodynamics", "domain": "Physics", "note_type": "deep-notes", "subdomain": "Statistical Physics"}"#,
            ),
            (
                "Quantum Mechanics",
                r#"{"topic": "Quantum Mechanics", "tags": ["topic/quantum-mechanics", "subject/quantum-mechanics", "domain/physics", "master-topic/quantum-mechanics", "subdomain/foundations", "concept/quantum-mechanics", "type/deep-notes"], "master_topic": "Quantum Mechanics", "domain": "Physics", "note_type": "deep-notes", "subdomain": "Foundations"}"#,
            ),
            (
                "Rust Programming",
                r#"{"topic": "Rust Programming", "tags": ["topic/rust-programming", "subject/software-engineering", "domain/computer-science", "master-topic/software-engineering", "subdomain/programming", "concept/rust-programming", "type/deep-notes"], "master_topic": "Software Engineering", "domain": "Computer Science", "note_type": "deep-notes", "subdomain": "Programming"}"#,
            ),
            (
                "Unknown X",
                r#"{"topic": "Unknown X", "tags": ["topic/unknown-x", "subject/general-knowledge", "domain/general", "master-topic/general-knowledge", "concept/unknown-x", "type/deep-notes"], "master_topic": "General Knowledge", "domain": "General", "note_type": "deep-notes"}"#,
            ),
        ] {
            let expected: Value = serde_json::from_str(expected).unwrap();
            assert_eq!(
                build_properties(topic, "deep-notes", None, None).unwrap(),
                expected,
                "{topic}"
            );
        }
    }
}
