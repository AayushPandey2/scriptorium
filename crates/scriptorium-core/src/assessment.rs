//! Objective-aligned questions and independently folded answers.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::collections::BTreeSet;
pub const TYPES: [&str; 10] = [
    "conceptual",
    "mathematical",
    "numerical",
    "algorithmic",
    "implementation",
    "comparison",
    "diagnosis",
    "experimental_design",
    "historical",
    "causal",
];
pub fn render_question(question: &Value) -> Result<String> {
    for key in ["id", "question", "hint", "solution", "type"] {
        ensure!(
            question[key].as_str().is_some_and(|s| !s.trim().is_empty()),
            "assessment needs nonempty {key}"
        );
    }
    ensure!(
        TYPES.contains(&question["type"].as_str().unwrap()),
        "unsupported assessment type"
    );
    let goals = question["objectives"]
        .as_array()
        .context("assessment needs objectives")?;
    ensure!(
        !goals.is_empty()
            && goals
                .iter()
                .all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty())),
        "assessment objectives must be nonempty IDs"
    );
    let quote = |text: &str| {
        text.lines()
            .map(|s| {
                if s.is_empty() {
                    ">".into()
                } else {
                    format!("> {s}")
                }
            })
            .collect::<Vec<String>>()
            .join("\n")
    };
    Ok(format!(
        "**Question ({}):** {}\n\n> [!hint]- Show hint\n{}\n\n> [!answer]- Show detailed answer\n{}",
        question["id"].as_str().unwrap(),
        question["question"].as_str().unwrap(),
        quote(question["hint"].as_str().unwrap()),
        quote(question["solution"].as_str().unwrap())
    ))
}
pub fn check_assessment(plan: &Value, chapter: &Value, body: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let questions: Vec<_> = plan["assessments"]
        .as_array()
        .map(|a| a.iter().filter(|q| q["chapter"] == chapter["id"]).collect())
        .unwrap_or_default();
    let finals: Vec<_> = questions
        .iter()
        .filter(|q| q["placement"].as_str().unwrap_or("chapter_end") == "chapter_end")
        .collect();
    if !(4..=5).contains(&finals.len()) {
        errors.push("comprehensive chapter needs four to five final questions".into());
    }
    if finals
        .iter()
        .filter_map(|q| q["type"].as_str())
        .collect::<BTreeSet<_>>()
        .len()
        < 3
    {
        errors.push("assessment lacks diversity across learning tasks".into());
    }
    let owned: BTreeSet<_> = chapter["concepts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let objectives: BTreeSet<_> = plan["concepts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["id"].as_str().is_some_and(|s| owned.contains(s)))
        .flat_map(|c| {
            c["objective_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .collect();
    let covered: BTreeSet<_> = questions
        .iter()
        .flat_map(|q| {
            q["objectives"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .collect();
    if covered != objectives {
        errors.push("assessment objective coverage is missing or refers to another chapter".into());
    }
    let mut ids = BTreeSet::new();
    for question in questions {
        if !ids.insert(question["id"].as_str().unwrap_or("")) {
            errors.push("assessment IDs must be unique".into());
        }
        match render_question(question) {
            Ok(rendered) => {
                if !body.contains(&rendered) {
                    errors.push("independently folded hint/answer absent from chapter".into());
                }
                let placement = question["placement"].as_str().unwrap_or("chapter_end");
                if placement != "chapter_end" {
                    match crate::composition::section_body(body, placement) {
                        Ok(section) if section.contains(&rendered) => (),
                        _ => errors
                            .push("section check absent from declared milestone heading".into()),
                    }
                }
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    errors
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn folded_hint_and_answer_are_independent() {
        let text=render_question(&json!({"id":"q1","question":"Why?","hint":"Consider the invariant.","solution":"The invariant is preserved by the update.","type":"conceptual","objectives":["c:objective:0"]})).unwrap();
        assert_eq!(text.matches("> [!").count(), 2);
        assert!(render_question(&json!({"id":"q"})).is_err());
    }
}
