//! Native repository, template and explanatory-structure checks.
use crate::orchestration::matches;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub fn validate_master_output(text: &str, master: &str, variant: Option<&str>) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return vec!["Output note is empty.".into()];
    }
    let mut errors = Vec::new();
    let h2 = text.lines().filter(|l| l.starts_with("## ")).count();
    let words = text.split_whitespace().count();
    let mut require = |yes: bool, message: &str| {
        if !yes {
            errors.push(message.to_string());
        }
    };
    match master {
        "M1" => {
            require(h2 >= 3, "M1 requires at least three structured H2 sections");
            require(
                words >= 120,
                "M1 is too sparse for first-principles mastery",
            );
        }
        "M2" => {
            require(h2 >= 2, "M2 requires at least two structured H2 sections");
            if variant == Some("lecture_order") {
                require(
                    matches(r"Source\s*&\s*Scope|Lecture|Chapter|Section", text),
                    "M2 lecture_order must preserve source scope context",
                );
            }
        }
        "M3" => {
            require(
                text.lines()
                    .any(|l| l.starts_with('|') || l.starts_with("- ") || l.starts_with("* "))
                    || text.contains("$$"),
                "M3 requires lookup tables, bullets or equations",
            );
            require(
                !text
                    .split("\n\n")
                    .filter(|p| !p.trim().starts_with(['#', '|', '-', '*', '>', '`', '$']))
                    .any(|p| p.split_whitespace().count() > 75),
                "M3 contains essay-length paragraphs",
            );
            if variant == Some("formula_sheet") {
                require(
                    text.contains(['$', '=']),
                    "Formula sheet requires equations",
                );
            }
        }
        "M4" => {
            require(
                matches(r"\b(Problem|Example|Exercise|Given)\b", text),
                "M4 requires concrete problems",
            );
            require(
                matches(r"\b(Step|Strategy|Method|Solution|Derivation)\b", text),
                "M4 requires explicit method and steps",
            );
            require(
                matches(
                    r"\b(Final Answer|Result|Check|Verify|Verification|Sanity)\b",
                    text,
                ),
                "M4 requires final answer and verification",
            );
        }
        "M5" => {
            require(text.contains("```"), "M5 requires fenced executable code");
            require(
                matches(
                    r"\b(Test|Verify|Output|Assert|Edge Case|Complexity|Invariant|Run)\b",
                    text,
                ),
                "M5 requires verification or edge-case checks",
            );
        }
        "M6" => {
            if variant == Some("flashcards") {
                require(
                    matches(r"\b(Front|Q:|Question|Card)\b", text)
                        && matches(r"\b(Back|A:|Answer|Reveal)\b", text),
                    "Flashcards require front/question and back/answer pairs",
                );
            } else {
                require(
                    matches(r"(?m)(^###\s+Q\d+|^\*\*Q\d+|^\d+\.\s+\*\*)", text),
                    "M6 requires numbered assessment questions",
                );
                require(
                    matches(
                        r">\s*\[!(tip|hint)\]-|\b(Answer Key|Solution|Rubric)\b",
                        text,
                    ),
                    "M6 must separate hints, solutions or rubrics",
                );
            }
        }
        "M7" => {
            if variant == Some("comparison_guide") {
                require(
                    text.contains('|'),
                    "Comparison guide requires a comparison matrix",
                );
            }
            if variant == Some("concept_map") {
                require(
                    text.contains("```mermaid"),
                    "Concept map requires a Mermaid diagram",
                );
            }
            require(
                matches(
                    r"(?m)(^\||```mermaid|\b(Comparison|Synthesis|Decision|Takeaway|Lineage|Verdict|Summary|Revision)\b)",
                    text,
                ),
                "M7 requires synthesis or revision structure",
            );
        }
        _ => errors.push(format!("Unknown master ID: {master}")),
    }
    errors
}
pub fn validate_registry(root: &Path) -> Result<Value> {
    let raw = std::fs::read_to_string(root.join("references/routing/master-registry.yaml"))?;
    let value: Value = serde_yaml_ng::from_str(&raw)?;
    let mut errors = Vec::new();
    let masters = value["masters"]
        .as_object()
        .context("masters mapping missing")?;
    ensure!(
        value["registry_version"] == 1,
        "unsupported registry version"
    );
    for id in ["M1", "M2", "M3", "M4", "M5", "M6", "M7"] {
        if !masters.contains_key(id) {
            errors.push(format!("missing master {id}"));
        }
    }
    let mut seen = BTreeSet::new();
    for (id, spec) in masters {
        for field in [
            "id",
            "route",
            "name",
            "path",
            "primary_intent",
            "output_goal",
        ] {
            if spec[field].as_str().is_none_or(|v| v.trim().is_empty()) {
                errors.push(format!("{id}: missing {field}"));
            }
        }
        if let Some(route) = spec["route"].as_str()
            && !seen.insert(route)
        {
            errors.push(format!("duplicate route {route}"));
        }
        if let Some(path) = spec["path"].as_str()
            && crate::validation::safe_path(root, Path::new(path), true).is_err()
        {
            errors.push(format!("{id}: missing or unsafe template path"));
        }
        if let Some(modules) = spec["default_modules"].as_array() {
            for module in modules {
                if let Some(id) = module.as_str()
                    && value["modules"].get(id).is_none()
                {
                    errors.push(format!("unknown default module {id}"));
                }
            }
        }
    }
    for field in ["variants", "subtemplates"] {
        if let Some(items) = value[field].as_object() {
            for (id, item) in items {
                if let Some(path) = item["path"].as_str()
                    && crate::validation::safe_path(root, Path::new(path), true).is_err()
                {
                    errors.push(format!("{field}/{id}: invalid template path"));
                }
                if field == "variants"
                    && item["parent_master"]
                        .as_str()
                        .is_none_or(|id| !masters.contains_key(id))
                {
                    errors.push(format!("variant {id}: unknown master"));
                }
            }
        }
    }
    let count = |name: &str| value[name].as_object().map(|m| m.len()).unwrap_or(0);
    Ok(
        json!({"valid":errors.is_empty(),"errors":errors,"masters_count":masters.len(),"variants_count":count("variants"),"subtemplates_count":count("subtemplates"),"modules_count":count("modules"),"commands_count":count("command_mappings"),"retired_commands_count":count("retired_commands")}),
    )
}
fn walk(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if [
                ".git",
                ".tools",
                "target",
                "__pycache__",
                ".pytest_cache",
                ".venv",
                ".test-deps",
                "cache",
            ]
            .contains(&entry.file_name().to_string_lossy().as_ref())
            {
                continue;
            }
            walk(&path, files)?;
        } else if kind.is_file() {
            files.push(path);
        }
    }
    Ok(())
}
fn decode(s: &str) -> Result<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            ensure!(i + 2 < b.len(), "invalid percent escape");
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16)?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Ok(String::from_utf8(out)?)
}
pub fn audit_repository(root: &Path) -> Result<Value> {
    let root = root.canonicalize()?;
    let mut files = Vec::new();
    walk(&root, &mut files)?;
    let mut errors = Vec::new();
    let mut links = 0;
    let pattern = regex::Regex::new(r"\]\(([^)]+)\)")?;
    for path in &files {
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let text = std::fs::read_to_string(path)?
            .trim_start_matches('\u{feff}')
            .to_owned();
        for capture in pattern.captures_iter(&text) {
            let target = capture.get(1).unwrap().as_str();
            if ["https:", "http:", "mailto:", "plugin:"]
                .iter()
                .any(|p| target.starts_with(p))
                || target.contains(['<', '>', '*'])
            {
                continue;
            }
            links += 1;
            let target = decode(target)?;
            let (name, anchor) = target.split_once('#').unwrap_or((&target, ""));
            let dest = if name.is_empty() {
                path.clone()
            } else {
                path.parent().unwrap().join(name)
            };
            if !dest.is_file() {
                errors.push(json!({"file":path.strip_prefix(&root)?,"target":target,"fix":"Resolve local target"}));
            } else if !anchor.is_empty() && dest.extension().is_some_and(|e| e == "md") {
                let doc = std::fs::read_to_string(dest)?;
                let valid = doc
                    .lines()
                    .filter(|l| l.starts_with('#'))
                    .any(|l| heading_slug(l.trim_start_matches('#').trim()) == anchor);
                if !valid {
                    errors.push(json!({"file":path.strip_prefix(&root)?,"target":target,"fix":"Resolve heading anchor"}));
                }
            }
        }
    }
    Ok(json!({"valid":errors.is_empty(),"files":files.len(),"local_links":links,"errors":errors}))
}
fn heading_slug(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == ' ' || *c == '-')
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace(' ', "-")
}
pub fn explanation_review(markdown: &str, audience: &str) -> Value {
    let mut issues = Vec::new();
    let mut add = |code: &str, description: &str| {
        issues.push(json!({"code":code,"severity":"major","description":description}))
    };
    if markdown.trim().is_empty() {
        add("CF-01", "Explanation is empty");
    }
    let body = markdown
        .split("---")
        .skip(2)
        .collect::<Vec<_>>()
        .join("---");
    let body = if body.is_empty() { markdown } else { &body };
    if body.split_whitespace().count() < 40 {
        add(
            "CF-12",
            "Explanation provides insufficient substance to assess a central question",
        );
    }
    if audience == "beginner"
        && !matches(
            r"\b(prerequisite|before|recall|intuit|example|means|defined)\b",
            body,
        )
    {
        add(
            "CF-02",
            "Beginner explanation lacks prerequisite bridge, definition or example",
        );
    }
    if body.contains("$$") && !matches(r"\b(where|denotes|represents|symbol|units|assum)\b", body) {
        add(
            "CF-05",
            "Equations need symbol meanings, assumptions and units",
        );
    }
    if body.contains("```")
        && !matches(
            r"\b(output|test|assert|example|trace|result|verify)\b",
            body,
        )
    {
        add(
            "CF-08",
            "Code needs a trace, output or verification context",
        );
    }
    json!({"passed":issues.is_empty(),"issues":issues,"hypothetical_review":true,"claims_measured_mastery":false,"scope":"Deterministic structural friction signals; scientific correctness and reader mastery require independent evidence"})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_master_output_fixture_parity() {
        let fixtures: Value =
            serde_json::from_str(include_str!("fixtures/master_outputs.json")).unwrap();
        for f in fixtures.as_array().unwrap() {
            assert_eq!(
                validate_master_output(
                    f["body"].as_str().unwrap(),
                    f["master"].as_str().unwrap(),
                    f["variant"].as_str()
                )
                .len() as u64,
                f["error_count"].as_u64().unwrap(),
                "{}",
                f["master"]
            );
        }
    }
    #[test]
    fn actual_registry_has_valid_owned_templates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let result = validate_registry(&root).unwrap();
        assert_eq!(result["valid"], true, "{}", result);
        assert_eq!(result["masters_count"], 7);
        assert_eq!(result["commands_count"], 4);
    }
    #[test]
    fn master_contracts_distinguish_products() {
        assert!(!validate_master_output("# Title", "M1", None).is_empty());
        assert!(validate_master_output("## Problem\nGiven x=1.\n## Solution\nStep 1 yields Result 2. Check by substitution.","M4",None).is_empty());
        assert!(
            validate_master_output("## Cheats\n- $x=2$", "M3", Some("formula_sheet")).is_empty()
        );
        assert!(!validate_master_output("## Essay\nAnswer is trivial.", "M6", None).is_empty());
    }
    #[test]
    fn audit_detects_broken_links_and_ignores_build_trees() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "[valid](valid.md#heading) [broken](missing.md)",
        )
        .unwrap();
        std::fs::write(dir.path().join("valid.md"), "# Heading\n").unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/ignored.md"), "[bad](missing.md)").unwrap();
        let report = audit_repository(dir.path()).unwrap();
        assert_eq!(report["errors"].as_array().unwrap().len(), 1);
    }
}
