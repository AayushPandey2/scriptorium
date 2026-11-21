//! Persisted configuration and author-outline approval. Draft text is supplied by
//! the authoring runtime; deterministic planning does not fabricate researched prose.
use crate::{
    metadata::slugify,
    orchestration::{RouteOptions, classify_scope, matches, route_request},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recommendation {
    pub recommended_master: String,
    pub recommended_variant: Option<String>,
    pub recommended_depth: String,
    pub recommended_organization: String,
    pub recommended_learning_objective: String,
    pub confidence: String,
    pub rationale: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preferences {
    pub learning_objective: String,
    pub master: String,
    pub variant: Option<String>,
    pub depth: String,
    pub organization: String,
    pub source_policy: String,
    pub preserve_source_order: bool,
    pub prior_knowledge: Value,
    pub emphasis: Vec<String>,
    pub language: Option<String>,
    pub scope_boundary: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopicNode {
    pub concept_id: String,
    pub title: String,
    pub parent_id: Option<String>,
    pub child_ids: Vec<String>,
    pub prerequisite_ids: Vec<String>,
    pub proposed_path: String,
    pub note_role: String,
    pub template_plan: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionPlan {
    pub collection_id: String,
    pub title: String,
    pub root_topic_id: String,
    pub organization: String,
    pub nodes: Vec<TopicNode>,
    pub planned_paths: Vec<String>,
    pub learning_order: Vec<String>,
    pub generation_order: Vec<String>,
    pub scope_summary: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub request_id: String,
    pub stage: String,
    pub original_intent: Value,
    pub recommendation: Recommendation,
    pub questions_asked: Vec<Value>,
    pub resolved_preferences: Option<Preferences>,
    pub collection_plan: Option<CollectionPlan>,
    pub source_sections: Vec<String>,
    pub source_policy: String,
    pub question_trace: Vec<Value>,
    pub persistence_mode: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionResponse {
    pub stage: String,
    pub message: String,
    pub ready_to_generate: bool,
    pub state: Session,
}
fn id(prefix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{now:x}-{:x}", SEQ.fetch_add(1, Ordering::Relaxed))
}
fn organization(request: &str, master: &str, depth: &str, topic: &str) -> String {
    if matches(
        r"\b(single[ -]note|single concept|one full note|one note|one file|all in one|in a single|concise|narrow)\b",
        request,
    ) {
        return "single_note".into();
    }
    if matches(
        r"\b(multiple linked notes|multi[ -]note|hierarchical|linked collection|separate chapters)\b",
        request,
    ) {
        return "hierarchical_collection".into();
    }
    if master == "M1"
        && (depth == "comprehensive"
            || classify_scope(topic, request).recommended_structure == "hierarchical_collection")
    {
        "hierarchical_collection".into()
    } else {
        "single_note".into()
    }
}
fn depth(request: &str) -> &'static str {
    if matches(r"\b(concise|quick|brief|short|narrow)\b", request) {
        "concise"
    } else if matches(
        r"\b(comprehensive|complete|full|deep|detailed|advanced)\b",
        request,
    ) {
        "comprehensive"
    } else {
        "standard"
    }
}
pub fn start(request: &str, has_source: bool) -> Result<Session> {
    start_with_sections(request, has_source, &[])
}
pub fn start_with_sections(
    request: &str,
    has_source: bool,
    sections: &[String],
) -> Result<Session> {
    let route = route_request(
        request,
        &RouteOptions {
            has_source,
            ..Default::default()
        },
    )?;
    let topic = route.topic.clone();
    let dep = depth(request);
    let org = organization(request, &route.master, dep, &topic);
    let rec = Recommendation {
        recommended_master: route.master.clone(),
        recommended_variant: route.variant.clone(),
        recommended_depth: dep.into(),
        recommended_organization: org,
        recommended_learning_objective: "conceptual_mastery".into(),
        confidence: if topic.is_empty() { "low" } else { "high" }.into(),
        rationale: vec![
            "Explicit format, organization and source constraints precede inferred scope".into(),
        ],
    };
    let mut questions = Vec::new();
    if topic.is_empty() {
        questions.push(json!({"id":"topic","question":"Which topic should the notes cover?"}));
    } else {
        if !matches(
            r"\b(concise|quick|brief|comprehensive|complete|full|deep|detailed|advanced|beginner)\b",
            request,
        ) {
            questions.push(json!({"id":"depth","question":"Choose concise, standard, or comprehensive depth.","options":["concise","standard","comprehensive"]}));
        }
        if !matches(
            r"\b(single|one file|multiple|hierarchical|all in one)\b",
            request,
        ) {
            questions.push(json!({"id":"organization","question":"Use a single note or a linked collection?","options":["single_note","hierarchical_collection"]}));
        }
    }
    Ok(Session {
        request_id: id("notes"),
        stage: if topic.is_empty() {
            "AWAITING_TOPIC"
        } else {
            "AWAITING_CONFIGURATION"
        }
        .into(),
        original_intent: json!({"raw_request":request,"topic":if topic.is_empty(){None}else{Some(topic)},"requested_artifact":route.master,"source_bindings":sections.iter().map(|s|json!({"section":s,"source":"supplied_source"})).collect::<Vec<_>>(),"inferred_scope":classify_scope(&route.topic,request),"migration_hint":route.migration_hint}),
        recommendation: rec,
        questions_asked: questions,
        resolved_preferences: None,
        collection_plan: None,
        source_sections: sections.to_vec(),
        source_policy: route.source_policy,
        question_trace: Vec::new(),
        persistence_mode: "in_memory_or_vault_session".into(),
    })
}
fn preferences(state: &Session, reply: &str) -> Result<Preferences> {
    let rec = &state.recommendation;
    let prior = state.resolved_preferences.as_ref();
    let mut master = prior
        .map(|p| p.master.clone())
        .unwrap_or_else(|| rec.recommended_master.clone());
    let mut variant = prior
        .map(|p| p.variant.clone())
        .unwrap_or_else(|| rec.recommended_variant.clone());
    let mut dep = prior
        .map(|p| p.depth.clone())
        .unwrap_or_else(|| rec.recommended_depth.clone());
    if matches(r"\b(concise|standard|comprehensive)\b", reply) {
        dep = depth(reply).into();
    }
    if matches(r"\b1[abc]\b|\b2[abc]\b", reply) {
        if reply.to_lowercase().contains("2c") {
            dep = "comprehensive".into();
        } else if reply.to_lowercase().contains("2a") {
            dep = "concise".into();
        } else if reply.to_lowercase().contains("2b") {
            dep = "standard".into();
        }
    }
    if matches(
        r"\b(master|template)\s*[:=]|\b(formula sheet|flashcards|worked solutions|implementation guide|comparison|study notes|cheat sheet)\b",
        reply,
    ) {
        let route = route_request(reply, &RouteOptions::default())?;
        master = route.master;
        variant = route.variant;
    }
    let topic = state.original_intent["topic"]
        .as_str()
        .unwrap_or("Untitled");
    let org = if matches(
        r"\b(single|one file|all in one|multiple|hierarchical|collection)\b",
        reply,
    ) {
        organization(reply, &master, &dep, topic)
    } else {
        prior
            .map(|p| p.organization.clone())
            .unwrap_or_else(|| rec.recommended_organization.clone())
    };
    let policy = if matches(
        r"\b(only my source|source[- ]only|strict|no outside)\b",
        reply,
    ) {
        "strict".into()
    } else {
        prior
            .map(|p| p.source_policy.clone())
            .unwrap_or_else(|| state.source_policy.clone())
    };
    Ok(Preferences {
        learning_objective: rec.recommended_learning_objective.clone(),
        master,
        variant,
        depth: dep,
        organization: org,
        source_policy: policy,
        preserve_source_order: matches(r"\b(source|lecture|chapter) order\b", reply),
        prior_knowledge: json!({}),
        emphasis: if matches(r"\bworked examples\b", reply) {
            vec!["worked_examples".into()]
        } else {
            Vec::new()
        },
        language: None,
        scope_boundary: "focused_curriculum".into(),
    })
}
fn node(
    title: String,
    id: String,
    parent: Option<String>,
    path: String,
    role: &str,
    master: &str,
) -> TopicNode {
    TopicNode {
        title,
        concept_id: id,
        parent_id: parent,
        child_ids: Vec::new(),
        prerequisite_ids: Vec::new(),
        proposed_path: path,
        note_role: role.into(),
        template_plan: json!({"master":master}),
    }
}
pub fn plan_collection(
    topic: &str,
    prefs: &Preferences,
    sections: &[String],
) -> Result<CollectionPlan> {
    ensure!(!topic.trim().is_empty(), "topic required");
    let slug = slugify(topic);
    let root_id = format!("{slug}:root");
    let collection_id = format!("collection-{slug}");
    let mut nodes = Vec::new();
    if prefs.organization == "single_note" {
        nodes.push(node(
            topic.into(),
            root_id.clone(),
            None,
            format!("{slug}.md"),
            "single",
            &prefs.master,
        ));
    } else {
        let titles: Vec<String> = if sections.is_empty() {
            vec![
                "Foundations and Prerequisites",
                "Core Concepts and Relationships",
                "Mechanisms and Worked Examples",
                "Applications and Failure Analysis",
                "Synthesis and Assessment",
            ]
            .into_iter()
            .map(|s| format!("{topic}: {s}"))
            .collect()
        } else {
            sections.to_vec()
        };
        ensure!(
            titles.len() <= 100,
            "collection outline exceeds 100 sections"
        );
        nodes.push(node(
            topic.into(),
            root_id.clone(),
            None,
            format!("{slug}/00 - Map of Contents.md"),
            "root_index",
            "index",
        ));
        for (i, title) in titles.iter().enumerate() {
            ensure!(!title.trim().is_empty(), "empty source section");
            let child = format!("{slug}:{}", i + 1);
            nodes[0].child_ids.push(child.clone());
            nodes.push(node(
                title.clone(),
                child,
                Some(root_id.clone()),
                format!("{slug}/{:02} - {}.md", i + 1, slugify(title)),
                "child",
                &prefs.master,
            ));
        }
    }
    let planned_paths = nodes.iter().map(|n| n.proposed_path.clone()).collect();
    let order = nodes
        .iter()
        .map(|n| n.concept_id.clone())
        .collect::<Vec<_>>();
    Ok(CollectionPlan {
        collection_id,
        title: topic.into(),
        root_topic_id: root_id,
        organization: prefs.organization.clone(),
        nodes,
        planned_paths,
        learning_order: order.clone(),
        generation_order: order,
        scope_summary: json!({"depth":prefs.depth,"outline_basis":if sections.is_empty(){"pedagogical chapter roles; author must supply topic-specific concept plan before generation"}else{"supplied source sections"},"source_policy":prefs.source_policy,"authored_content_required":true}),
    })
}
pub fn prompt(state: &Session) -> String {
    if state.stage == "AWAITING_TOPIC" {
        "Which topic should the notes cover?".into()
    } else {
        format!(
            "Recommended: {}, {} depth, {}. Confirm these recommendations or provide corrections. {}",
            state.recommendation.recommended_master,
            state.recommendation.recommended_depth,
            state.recommendation.recommended_organization,
            state
                .questions_asked
                .iter()
                .filter_map(|q| q["question"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}
pub fn respond(mut state: Session, reply: &str) -> Result<SessionResponse> {
    ensure!(!reply.trim().is_empty(), "reply cannot be empty");
    let low = reply.trim().trim_end_matches('.').to_lowercase();
    if ["cancel", "stop", "abort", "never mind", "start over"].contains(&low.as_str()) {
        state.stage = "CANCELLED".into();
        return Ok(response(state, "Cancelled the pending request."));
    }
    if reply.trim_start().starts_with("/scribe-notes") {
        return Ok(response(start(reply, false)?, "Started a new request."));
    }
    match state.stage.as_str() {
        "AWAITING_TOPIC" => {
            let mut next = start(reply, false)?;
            next.request_id = state.request_id;
            return Ok(response(
                next,
                "Topic received. Review the recommended configuration.",
            ));
        }
        "AWAITING_CONFIGURATION" => {
            let prefs = preferences(&state, reply)?;
            let topic = state.original_intent["topic"]
                .as_str()
                .context("session missing topic")?;
            let plan = plan_collection(topic, &prefs, &state.source_sections)?;
            state.stage =
                if plan.organization == "hierarchical_collection" && plan.nodes.len() >= 4 {
                    "AWAITING_PLAN_APPROVAL"
                } else {
                    "COLLECTION_PLAN_READY"
                }
                .into();
            state.collection_plan = Some(plan);
            state.resolved_preferences = Some(prefs);
        }
        "AWAITING_PLAN_APPROVAL" => {
            if matches(
                r"\b(proceed|approve|looks? good|yes|generate|go ahead|confirm)\b",
                reply,
            ) {
                state.stage = "COLLECTION_PLAN_READY".into();
            } else if matches(
                r"\b(actually|change to|make it|single|multiple|depth)\b",
                reply,
            ) {
                let prefs = preferences(&state, reply)?;
                let topic = state.original_intent["topic"]
                    .as_str()
                    .context("session missing topic")?;
                state.collection_plan =
                    Some(plan_collection(topic, &prefs, &state.source_sections)?);
                if prefs.organization == "single_note" {
                    state.stage = "COLLECTION_PLAN_READY".into();
                }
                state.resolved_preferences = Some(prefs);
            } else if let Some(rest) = reply.trim().strip_prefix("outline:") {
                let sections: Vec<_> = rest
                    .split(';')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                ensure!(
                    !sections.is_empty(),
                    "outline needs section titles separated by semicolons"
                );
                state.source_sections = sections;
                let prefs = state
                    .resolved_preferences
                    .as_ref()
                    .context("missing preferences")?;
                state.collection_plan = Some(plan_collection(
                    state.original_intent["topic"]
                        .as_str()
                        .context("missing topic")?,
                    prefs,
                    &state.source_sections,
                )?);
            } else {
                let plan = state
                    .collection_plan
                    .as_ref()
                    .context("missing collection plan")?;
                state.collection_plan = Some(modify_plan(plan, reply)?);
            }
        }
        "COLLECTION_PLAN_READY" | "CANCELLED" => (),
        _ => bail!("unsupported session stage {}", state.stage),
    }
    state
        .question_trace
        .push(json!({"reply":reply,"resulting_stage":state.stage}));
    Ok(response(
        state,
        "Configuration and outline updated. Draft generation requires the approved plan and authored evidence.",
    ))
}
fn response(state: Session, message: &str) -> SessionResponse {
    SessionResponse {
        stage: state.stage.clone(),
        ready_to_generate: state.stage == "COLLECTION_PLAN_READY",
        message: message.into(),
        state,
    }
}
/// Apply bounded explicit outline edits without discarding session preferences.
pub fn modify_plan(original: &CollectionPlan, instruction: &str) -> Result<CollectionPlan> {
    let mut plan = original.clone();
    let input = instruction.trim();
    let lower = input.to_lowercase();
    let select = |nodes: &[TopicNode], name: &str| -> Result<usize> {
        let name = name.trim().trim_matches(['"', '\'']).to_lowercase();
        let found: Vec<_> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                n.concept_id != original.root_topic_id
                    && (n.title.to_lowercase() == name || n.title.to_lowercase().contains(&name))
            })
            .map(|(i, _)| i)
            .collect();
        ensure!(
            found.len() == 1,
            "outline target must identify exactly one chapter"
        );
        Ok(found[0])
    };
    let mut removed = Vec::new();
    if let Some(title) = lower
        .strip_prefix("remove ")
        .or_else(|| lower.strip_prefix("drop "))
        .or_else(|| lower.strip_prefix("skip "))
        .or_else(|| lower.strip_prefix("exclude "))
    {
        let index = select(
            &plan.nodes,
            title
                .trim_end_matches(" section")
                .trim_end_matches(" group"),
        )?;
        removed.push(plan.nodes[index].concept_id.clone());
        removed.extend(plan.nodes[index].child_ids.clone());
    } else if lower.starts_with("add ") {
        let title = input[4..].trim().trim_start_matches("a note on ");
        ensure!(!title.is_empty(), "new chapter needs title");
        let root = plan
            .nodes
            .iter()
            .position(|n| n.concept_id == plan.root_topic_id)
            .context("missing root")?;
        let cid = format!("{}:{}", plan.root_topic_id, slugify(title));
        ensure!(
            !plan.nodes.iter().any(|n| n.concept_id == cid),
            "chapter already exists"
        );
        let folder = Path::new(&plan.nodes[root].proposed_path)
            .parent()
            .context("root needs parent folder")?
            .to_string_lossy()
            .replace('\\', "/");
        let master = plan
            .nodes
            .iter()
            .find(|n| n.note_role == "child")
            .and_then(|n| n.template_plan["master"].as_str())
            .unwrap_or("M1")
            .to_owned();
        plan.nodes[root].child_ids.push(cid.clone());
        plan.nodes.push(node(
            title.into(),
            cid,
            Some(plan.root_topic_id.clone()),
            format!("{folder}/{}.md", slugify(title)),
            "child",
            &master,
        ));
    } else if lower.starts_with("rename ") {
        let split = lower
            .rfind(" to ")
            .context("rename syntax: rename old title to new title")?;
        let index = select(&plan.nodes, &input[7..split])?;
        let title = input[split + 4..].trim();
        ensure!(!title.is_empty(), "new title required");
        let folder = Path::new(&plan.nodes[index].proposed_path)
            .parent()
            .context("chapter needs parent folder")?
            .to_string_lossy()
            .replace('\\', "/");
        plan.nodes[index].title = title.into();
        plan.nodes[index].proposed_path = format!("{folder}/{}.md", slugify(title));
    } else if lower.starts_with("split ") {
        let split = lower
            .find(" into ")
            .context("split syntax: split title into first; second")?;
        let index = select(&plan.nodes, &input[6..split])?;
        let titles: Vec<_> = input[split + 6..]
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        ensure!(
            titles.len() >= 2,
            "split requires at least two semicolon-separated titles"
        );
        let old = plan.nodes[index].clone();
        let folder = Path::new(&old.proposed_path)
            .parent()
            .context("chapter needs folder")?
            .to_string_lossy()
            .replace('\\', "/");
        plan.nodes[index].title = titles[0].into();
        plan.nodes[index].proposed_path = format!("{folder}/{}.md", slugify(titles[0]));
        let mut prior = old.concept_id.clone();
        for title in titles.iter().skip(1) {
            let cid = format!("{}:{}", old.concept_id, slugify(title));
            ensure!(
                !plan.nodes.iter().any(|n| n.concept_id == cid),
                "duplicate split chapter"
            );
            let mut new = node(
                (*title).into(),
                cid.clone(),
                old.parent_id.clone(),
                format!("{folder}/{}.md", slugify(title)),
                "child",
                old.template_plan["master"].as_str().unwrap_or("M1"),
            );
            new.prerequisite_ids.push(prior);
            prior = cid.clone();
            if let Some(parent) = &old.parent_id {
                let p = plan
                    .nodes
                    .iter_mut()
                    .find(|n| &n.concept_id == parent)
                    .context("missing parent")?;
                p.child_ids.push(cid);
            }
            plan.nodes.push(new);
        }
    } else if lower.starts_with("merge ") {
        let into = lower
            .rfind(" into ")
            .context("merge syntax: merge first and second into title")?;
        let pair = &input[6..into];
        let and = pair
            .to_lowercase()
            .find(" and ")
            .context("merge requires two chapter titles")?;
        let first = select(&plan.nodes, &pair[..and])?;
        let second = select(&plan.nodes, &pair[and + 5..])?;
        ensure!(first != second, "cannot merge a chapter with itself");
        let title = input[into + 6..].trim();
        ensure!(!title.is_empty(), "merged title required");
        let oldid = plan.nodes[second].concept_id.clone();
        let id = plan.nodes[first].concept_id.clone();
        let folder = Path::new(&plan.nodes[first].proposed_path)
            .parent()
            .context("chapter needs folder")?
            .to_string_lossy()
            .replace('\\', "/");
        let dependencies = plan.nodes[second].prerequisite_ids.clone();
        plan.nodes[first].title = title.into();
        plan.nodes[first].proposed_path = format!("{folder}/{}.md", slugify(title));
        plan.nodes[first].prerequisite_ids.extend(dependencies);
        for n in &mut plan.nodes {
            for p in &mut n.prerequisite_ids {
                if *p == oldid {
                    *p = id.clone();
                }
            }
        }
        removed.push(oldid);
    } else if let Some(title) = lower
        .strip_prefix("generate only ")
        .or_else(|| lower.strip_prefix("only "))
        .or_else(|| lower.strip_prefix("just "))
    {
        let index = select(&plan.nodes, title.trim_end_matches(" section"))?;
        let selected = plan.nodes[index].concept_id.clone();
        removed.extend(
            plan.nodes
                .iter()
                .filter(|n| n.concept_id != selected && n.concept_id != plan.root_topic_id)
                .map(|n| n.concept_id.clone()),
        );
    } else {
        bail!("Use add/remove/rename/split/merge, 'only title', or 'outline: title; title'");
    }
    plan.nodes.retain(|n| !removed.contains(&n.concept_id));
    let ids: std::collections::BTreeSet<_> =
        plan.nodes.iter().map(|n| n.concept_id.clone()).collect();
    for n in &mut plan.nodes {
        n.child_ids.retain(|id| ids.contains(id));
        n.prerequisite_ids
            .retain(|id| ids.contains(id) && *id != n.concept_id);
        n.prerequisite_ids.sort();
        n.prerequisite_ids.dedup();
    }
    let paths: std::collections::BTreeSet<_> = plan
        .nodes
        .iter()
        .map(|n| n.proposed_path.to_lowercase())
        .collect();
    ensure!(
        paths.len() == plan.nodes.len(),
        "outline edit creates colliding note paths"
    );
    ensure!(
        plan.nodes.len() > 1 || plan.organization == "single_note",
        "collection cannot contain only its index"
    );
    plan.planned_paths = plan.nodes.iter().map(|n| n.proposed_path.clone()).collect();
    plan.learning_order = plan
        .nodes
        .iter()
        .filter(|n| n.note_role == "child" || n.note_role == "single")
        .map(|n| n.concept_id.clone())
        .collect();
    plan.generation_order = plan.learning_order.clone();
    plan.generation_order.push(plan.root_topic_id.clone());
    plan.scope_summary["total_notes"] = json!(plan.nodes.len());
    Ok(plan)
}
fn identifier(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "invalid session identifier"
    );
    Ok(())
}
pub fn save(vault: &Path, state: &Session) -> Result<PathBuf> {
    identifier(&state.request_id)?;
    let root = crate::validation::safe_path(vault, Path::new(".scriptorium/sessions"), false)?;
    std::fs::create_dir_all(&root)?;
    let root = crate::validation::safe_path(vault, Path::new(".scriptorium/sessions"), true)?;
    let path = root.join(format!("{}.json", state.request_id));
    ensure!(
        !path.exists() || !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "session symlink rejected"
    );
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    temp.write_all(&serde_json::to_vec_pretty(state)?)?;
    temp.as_file().sync_all()?;
    temp.persist(&path)?;
    Ok(path)
}
pub fn load(vault: &Path, id: &str) -> Result<Session> {
    identifier(id)?;
    let path = crate::validation::safe_path(
        vault,
        &PathBuf::from(format!(".scriptorium/sessions/{id}.json")),
        true,
    )?;
    ensure!(
        std::fs::metadata(&path)?.len() <= 8 * 1024 * 1024,
        "session exceeds size limit"
    );
    let state: Session = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(state.request_id == id, "session identity mismatch");
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_scope_preserves_explicit_styles_and_single_overrides() {
        for (request, master, org) in [
            (
                "comprehensive Kirchhoff circuit notes with units and examples",
                "M1",
                "hierarchical_collection",
            ),
            (
                "comprehensive structured study notes on parliament",
                "M2",
                "single_note",
            ),
            (
                "comprehensive formula sheet for mechanics",
                "M3",
                "single_note",
            ),
            (
                "complete notes about rivers in one file",
                "M1",
                "single_note",
            ),
            ("quick binary search reminder", "M3", "single_note"),
        ] {
            let s = start(request, false).unwrap();
            assert_eq!(s.recommendation.recommended_master, master);
            assert_eq!(s.recommendation.recommended_organization, org);
            let r = respond(s, "use your recommendations").unwrap();
            assert_eq!(r.state.collection_plan.unwrap().organization, org);
        }
    }
    #[test]
    fn state_transitions_cancel_approval_and_persistence() {
        let s = start("/scribe-notes", false).unwrap();
        assert_eq!(s.stage, "AWAITING_TOPIC");
        let r = respond(
            s,
            "comprehensive notes on medieval law using only my source",
        )
        .unwrap();
        let r = respond(r.state, "use your recommendations").unwrap();
        assert_eq!(r.stage, "AWAITING_PLAN_APPROVAL");
        assert_eq!(
            r.state.resolved_preferences.as_ref().unwrap().source_policy,
            "strict"
        );
        let r = respond(
            r.state,
            "outline: Foundations; Rules; Applications; Sources; Assessment",
        )
        .unwrap();
        let r = respond(r.state, "approve").unwrap();
        assert!(r.ready_to_generate);
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &r.state).unwrap();
        assert_eq!(
            load(dir.path(), &r.state.request_id).unwrap().stage,
            "COLLECTION_PLAN_READY"
        );
        assert!(load(dir.path(), "../escape").is_err());
        assert_eq!(respond(r.state, "cancel").unwrap().stage, "CANCELLED");
    }
    #[test]
    fn outline_edits_preserve_graph_and_preferences() {
        let s = start_with_sections(
            "comprehensive notes about circuits",
            false,
            &[
                "Basics".into(),
                "Components".into(),
                "Analysis".into(),
                "Examples".into(),
                "Assessment".into(),
            ],
        )
        .unwrap();
        let r = respond(s, "use your recommendations").unwrap();
        let mut p = r.state.collection_plan.unwrap();
        p = modify_plan(&p, "split Analysis into DC; AC").unwrap();
        assert!(p.nodes.iter().any(|n| n.title == "AC"));
        p = modify_plan(&p, "merge DC and AC into Circuit Analysis").unwrap();
        assert!(!p.nodes.iter().any(|n| n.title == "AC"));
        p = modify_plan(&p, "rename Basics to Foundations").unwrap();
        assert!(p.nodes.iter().any(|n| n.title == "Foundations"));
        p = modify_plan(&p, "add Measurement").unwrap();
        p = modify_plan(&p, "remove Measurement").unwrap();
        assert!(!p.nodes.iter().any(|n| n.title == "Measurement"));
        assert!(modify_plan(&p, "remove missing chapter").is_err());
    }
}
