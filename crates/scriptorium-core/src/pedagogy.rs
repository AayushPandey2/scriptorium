//! Native educational planning. All claims about authored material remain review-bound.
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const DIMENSIONS: [&str; 7] = [
    "conceptual",
    "mathematical",
    "algorithmic",
    "implementation",
    "research",
    "visual",
    "analytical",
];
pub fn objective_kind(goal: &str, explicit: Option<&str>) -> Result<String> {
    let allowed = [
        "historical",
        "algorithmic",
        "mathematical",
        "causal",
        "implementation",
        "reproduction",
        "synthesis",
        "conceptual",
        "visual",
    ];
    if let Some(kind) = explicit {
        ensure!(allowed.contains(&kind), "unknown semantic objective kind");
        return Ok(kind.into());
    }
    let words: BTreeSet<_> = goal
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .collect();
    let has = |terms: &[&str]| terms.iter().any(|s| words.contains(*s));
    Ok(
        if has(&["history", "historical", "evolution", "chronology"]) {
            "historical"
        } else if has(&["reproduce", "replicate", "reproduction"]) {
            "reproduction"
        } else if has(&["derive", "prove", "derivation", "proof"]) {
            "mathematical"
        } else if has(&["implement", "debug", "code"]) {
            "implementation"
        } else if has(&["trace", "execute", "simulate"])
            && has(&[
                "state",
                "states",
                "iteration",
                "algorithm",
                "execution",
                "loop",
                "token",
                "tokens",
            ])
        {
            "algorithmic"
        } else if has(&["cause", "causal", "why"]) {
            "causal"
        } else if has(&["compare", "synthesize", "connect", "analyze"]) {
            "synthesis"
        } else if has(&["visualize", "plot", "draw"]) {
            "visual"
        } else {
            "conceptual"
        }
        .into(),
    )
}
pub fn allocate_depth(concept: &Value, source_policy: &str) -> Result<Value> {
    ensure!(
        ["strict", "source_first", "adaptive"].contains(&source_policy),
        "invalid source policy"
    );
    let mut levels: BTreeMap<String, u64> = DIMENSIONS.iter().map(|s| (s.to_string(), 0)).collect();
    let mut reasons: BTreeMap<String, Vec<String>> = DIMENSIONS
        .iter()
        .map(|s| (s.to_string(), Vec::new()))
        .collect();
    let names = [
        "omitted",
        "orientation",
        "working",
        "advanced",
        "comprehensive",
        "research",
    ];
    if let Some(requested) = concept.get("depth") {
        for (dim, value) in requested.as_object().context("depth must be object")? {
            ensure!(levels.contains_key(dim), "unknown depth dimension");
            let level = value
                .as_u64()
                .or_else(|| {
                    value
                        .as_str()
                        .and_then(|s| names.iter().position(|x| *x == s).map(|i| i as u64))
                })
                .context("invalid depth level")?;
            ensure!(level <= 5, "depth must be 0..5");
            levels.insert(dim.clone(), level);
            reasons
                .get_mut(dim)
                .unwrap()
                .push("explicit concept requirement".into());
        }
    }
    let mut raise = |dim: &str, min: u64, reason: String| {
        let level = levels.get_mut(dim).unwrap();
        *level = (*level).max(min);
        reasons.get_mut(dim).unwrap().push(reason);
    };
    for (i, goal) in concept["objectives"]
        .as_array()
        .context("objectives must be array")?
        .iter()
        .enumerate()
    {
        let goal = goal.as_str().context("objective must be text")?;
        ensure!(!goal.trim().is_empty(), "empty objective");
        match goal
            .split_whitespace()
            .next()
            .unwrap()
            .to_lowercase()
            .as_str()
        {
            "explain" => raise("conceptual", 2, format!("learning objective: {goal}")),
            "derive" => raise("mathematical", 3, format!("learning objective: {goal}")),
            "prove" => raise("mathematical", 5, format!("learning objective: {goal}")),
            "implement" => raise("implementation", 3, format!("learning objective: {goal}")),
            "compare" | "analyze" => raise("analytical", 3, format!("learning objective: {goal}")),
            "visualize" => raise("visual", 2, format!("learning objective: {goal}")),
            _ => (),
        }
        let kind = objective_kind(goal, concept["objective_kinds"][i.to_string()].as_str())?;
        let pair = match kind.as_str() {
            "historical" => Some(("conceptual", 2)),
            "algorithmic" => Some(("algorithmic", 3)),
            "mathematical" => Some(("mathematical", 3)),
            "implementation" => Some(("implementation", 3)),
            "reproduction" => Some(("research", 4)),
            _ => None,
        };
        if let Some((dim, n)) = pair {
            raise(dim, n, format!("semantic objective ({kind}): {goal}"));
        }
    }
    for (flag, dim, n) in [
        ("difficult", "conceptual", 3),
        ("missing_prerequisite", "conceptual", 3),
        ("conflicting_evidence", "research", 4),
        ("novel", "research", 3),
        ("high_consequence", "research", 4),
        ("spatial", "visual", 2),
    ] {
        if concept["signals"][flag] == true {
            raise(dim, n, flag.into());
        }
    }
    Ok(
        json!({"levels":levels,"reasons":reasons,"external_retrieval_permitted":source_policy!="strict","unresolved":if source_policy=="strict"&&concept["signals"]["source_gap"]==true{vec!["source gap: external retrieval prohibited"]}else{vec![]}}),
    )
}
pub fn capabilities() -> Result<Value> {
    let mut registry: Value = serde_json::from_str(include_str!(
        "../../../references/workflows/notes-creation/instructional-registry.json"
    ))?;
    let base = registry["capability_contract"]
        .as_object()
        .context("registry contract missing")?
        .clone();
    for value in registry["capabilities"]
        .as_object_mut()
        .context("registry capabilities missing")?
        .values_mut()
    {
        let mut merged = base.clone();
        merged.extend(value.as_object().context("invalid capability")?.clone());
        merged.insert("explanation_strategy".into(), value["stages"].clone());
        *value = Value::Object(merged);
    }
    Ok(registry["capabilities"].clone())
}
pub fn resolve_capabilities(selected: &[String]) -> Result<Vec<String>> {
    fn visit(
        id: &str,
        specs: &Value,
        active: &mut BTreeSet<String>,
        done: &mut Vec<String>,
    ) -> Result<()> {
        ensure!(specs.get(id).is_some(), "unknown capability {id}");
        if done.iter().any(|s| s == id) {
            return Ok(());
        }
        ensure!(active.insert(id.into()), "capability cycle");
        for dep in specs[id]["dependencies"]
            .as_array()
            .context("capability dependencies missing")?
        {
            visit(
                dep.as_str().context("invalid dependency")?,
                specs,
                active,
                done,
            )?;
        }
        active.remove(id);
        done.push(id.into());
        Ok(())
    }
    let specs = capabilities()?;
    let mut done = Vec::new();
    let mut active = BTreeSet::new();
    for id in selected {
        visit(id, &specs, &mut active, &mut done)?;
    }
    Ok(done)
}
pub fn arithmetic(expression: &str, variables: &Value) -> Result<f64> {
    ensure!(
        expression.len() <= 2000,
        "arithmetic expression budget exceeded"
    );
    ensure!(
        expression
            .chars()
            .filter(|c| !c.is_alphanumeric() && !c.is_whitespace())
            .count()
            <= 200,
        "arithmetic token budget exceeded"
    );
    let parsed: meval::Expr = expression.replace("**", "^").parse()?;
    let mut ctx = meval::Context::new();
    if let Some(vars) = variables.as_object() {
        for (name, value) in vars {
            let n = value.as_f64().context("variable must be numeric")?;
            ensure!(n.is_finite(), "nonfinite variable");
            ctx.var(name, n);
        }
    }
    let value = parsed.eval_with_context(ctx)?;
    ensure!(value.is_finite(), "nonfinite arithmetic result");
    Ok(value)
}
pub fn check_numerical(record: &Value) -> Vec<String> {
    let result: Result<()> = (|| {
        let actual = arithmetic(
            record["expression"]
                .as_str()
                .context("missing numerical expression")?,
            &record["variables"],
        )?;
        let expected = record["expected"]
            .as_f64()
            .context("missing expected result")?;
        let atol = record["atol"].as_f64().unwrap_or(1e-12);
        let rtol = record["rtol"].as_f64().unwrap_or(1e-9);
        ensure!(
            expected.is_finite()
                && atol.is_finite()
                && rtol.is_finite()
                && atol >= 0.
                && rtol >= 0.,
            "invalid comparison tolerance or expected value"
        );
        let mode = record["comparison"].as_str().unwrap_or("allclose");
        ensure!(
            ["exact", "allclose"].contains(&mode),
            "unsupported equality convention"
        );
        ensure!(
            if mode == "exact" {
                actual == expected
            } else {
                (actual - expected).abs() <= atol.max(rtol * actual.abs().max(expected.abs()))
            },
            "counterexample: observed {actual}, expected {expected}"
        );
        ensure!(
            record["epsilon"].as_f64().unwrap_or(0.) == 0.
                || record["stabilizer_effect"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty()),
            "stabilizer effect needs explanation"
        );
        Ok(())
    })();
    result
        .err()
        .map(|e| vec![e.to_string()])
        .unwrap_or_default()
}
pub fn explanation_blueprint(
    topic: &str,
    master: &str,
    audience: &str,
    source_mode: &str,
) -> Result<Value> {
    ensure!(!topic.trim().is_empty(), "topic required");
    ensure!(
        ["M1", "M2", "M3", "M4", "M5", "M6", "M7"].contains(&master),
        "invalid master"
    );
    ensure!(
        [
            "beginner",
            "intermediate",
            "advanced",
            "cross_domain_expert"
        ]
        .contains(&audience),
        "invalid audience"
    );
    ensure!(
        ["strict", "source_first", "adaptive"].contains(&source_mode),
        "invalid source mode"
    );
    let stages = if audience == "beginner" {
        vec![
            "prerequisite_bridge",
            "motivation",
            "intuition",
            "definition",
            "worked_example",
            "interpretation",
            "limits",
            "active_recall",
        ]
    } else {
        vec![
            "motivation",
            "definition",
            "mechanism",
            "intermediate_reasoning",
            "worked_example",
            "assumptions",
            "limits",
            "connections",
            "active_recall",
        ]
    };
    let levels: Map<String, Value> = (1..=7)
        .map(|i| {
            (
                format!("eis_{i:02}"),
                json!(if master == "M3" {
                    "compact"
                } else {
                    "standard"
                }),
            )
        })
        .collect();
    Ok(
        json!({"topic":topic,"master":master,"audience":audience,"source_mode":source_mode,"central_question":format!("How does {topic} work, under which assumptions, and when is it useful?"),"sequence":stages,"activation_levels":levels,"external_facts_permitted":source_mode!="strict","review_required":true}),
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rational(i128, i128);
impl Rational {
    fn new(mut n: i128, mut d: i128) -> Result<Self> {
        ensure!(d != 0, "division by zero");
        if d < 0 {
            n = n.checked_neg().context("rational overflow")?;
            d = d.checked_neg().context("rational overflow")?;
        }
        let (mut a, mut b) = (n.unsigned_abs(), d as u128);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let g = a.max(1) as i128;
        Ok(Self(n / g, d / g))
    }
    fn add(self, b: Self) -> Result<Self> {
        Self::new(
            self.0
                .checked_mul(b.1)
                .and_then(|a| b.0.checked_mul(self.1).and_then(|b| a.checked_add(b)))
                .context("rational overflow")?,
            self.1.checked_mul(b.1).context("rational overflow")?,
        )
    }
    fn mul(self, b: Self) -> Result<Self> {
        Self::new(
            self.0.checked_mul(b.0).context("rational overflow")?,
            self.1.checked_mul(b.1).context("rational overflow")?,
        )
    }
    fn parse(value: &str) -> Result<Self> {
        let (mantissa, exponent) = value
            .split_once(['e', 'E'])
            .map(|(a, b)| (a, b.parse::<i32>()))
            .unwrap_or((value, Ok(0)));
        let exponent = exponent?;
        ensure!(exponent.abs() <= 18, "decimal exponent budget exceeded");
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        ensure!(fraction.len() <= 18, "decimal precision budget exceeded");
        let digits = format!("{whole}{fraction}");
        let n = digits.parse::<i128>()?;
        let scale = fraction.len() as i32 - exponent;
        if scale >= 0 {
            Self::new(
                n,
                10i128
                    .checked_pow(scale as u32)
                    .context("rational overflow")?,
            )
        } else {
            Self::new(
                n.checked_mul(
                    10i128
                        .checked_pow((-scale) as u32)
                        .context("rational overflow")?,
                )
                .context("rational overflow")?,
                1,
            )
        }
    }
}
type Polynomial = BTreeMap<Vec<String>, Rational>;
fn poly_add(mut a: Polynomial, b: Polynomial) -> Result<Polynomial> {
    for (k, v) in b {
        let sum = a.get(&k).copied().unwrap_or(Rational(0, 1)).add(v)?;
        if sum.0 == 0 {
            a.remove(&k);
        } else {
            a.insert(k, sum);
        }
    }
    ensure!(a.len() <= 4096, "polynomial term budget exceeded");
    Ok(a)
}
fn poly_mul(a: &Polynomial, b: &Polynomial) -> Result<Polynomial> {
    ensure!(
        a.len().saturating_mul(b.len()) <= 4096,
        "polynomial expansion budget exceeded"
    );
    let mut result = Polynomial::new();
    for (ka, va) in a {
        for (kb, vb) in b {
            let mut key = ka.clone();
            key.extend(kb.clone());
            key.sort();
            result = poly_add(result, BTreeMap::from([(key, va.mul(*vb)?)]))?;
        }
    }
    Ok(result)
}
fn polynomial(expression: &str) -> Result<Polynomial> {
    ensure!(expression.len() <= 2000, "polynomial expression too large");
    let source = expression.replace("**", "^");
    let mut tokens = Vec::<String>::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if c.is_ascii_digit() || c == '.' {
            i += 1;
            while i < bytes.len() && ((bytes[i] as char).is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && b"eE".contains(&bytes[i]) {
                i += 1;
                if i < bytes.len() && b"+-".contains(&bytes[i]) {
                    i += 1;
                }
                while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                    i += 1;
                }
            }
        } else if c.is_ascii_alphabetic() || c == '_' {
            i += 1;
            while i < bytes.len()
                && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
        } else {
            ensure!(
                "+-*/^()".contains(c),
                "expression outside polynomial verifier domain"
            );
            i += 1;
        }
        tokens.push(source[start..i].into());
        ensure!(tokens.len() <= 200, "polynomial token budget exceeded");
    }
    struct Parse {
        tokens: Vec<String>,
        at: usize,
        depth: usize,
    }
    impl Parse {
        fn is(&self, s: &str) -> bool {
            self.tokens.get(self.at).is_some_and(|v| v == s)
        }
        fn expression(&mut self) -> Result<Polynomial> {
            self.depth += 1;
            ensure!(self.depth <= 32, "polynomial nesting budget exceeded");
            let mut a = self.term()?;
            while self.is("+") || self.is("-") {
                let minus = self.is("-");
                self.at += 1;
                let mut b = self.term()?;
                if minus {
                    for v in b.values_mut() {
                        v.0 = v.0.checked_neg().context("rational overflow")?;
                    }
                }
                a = poly_add(a, b)?;
            }
            self.depth -= 1;
            Ok(a)
        }
        fn term(&mut self) -> Result<Polynomial> {
            let mut a = self.unary()?;
            while self.is("*") || self.is("/") {
                let divide = self.is("/");
                self.at += 1;
                let b = self.unary()?;
                if divide {
                    ensure!(
                        b.len() == 1 && b.contains_key(&Vec::new()),
                        "division by variables unsupported"
                    );
                    let v = b[&Vec::new()];
                    ensure!(v.0 != 0, "division by zero");
                    a = poly_mul(
                        &a,
                        &BTreeMap::from([(Vec::new(), Rational::new(v.1, v.0)?)]),
                    )?;
                } else {
                    a = poly_mul(&a, &b)?;
                }
            }
            Ok(a)
        }
        fn unary(&mut self) -> Result<Polynomial> {
            if self.is("+") || self.is("-") {
                let negative = self.is("-");
                self.at += 1;
                let mut a = self.unary()?;
                if negative {
                    for v in a.values_mut() {
                        v.0 = v.0.checked_neg().context("rational overflow")?;
                    }
                }
                Ok(a)
            } else {
                self.power()
            }
        }
        fn power(&mut self) -> Result<Polynomial> {
            let a = self.atom()?;
            if self.is("^") {
                self.at += 1;
                let token = self.tokens.get(self.at).context("missing exponent")?;
                let power = token
                    .parse::<u32>()
                    .context("only bounded nonnegative integer powers supported")?;
                ensure!(power <= 12, "power exceeds polynomial budget");
                self.at += 1;
                let mut result = BTreeMap::from([(Vec::new(), Rational(1, 1))]);
                for _ in 0..power {
                    result = poly_mul(&result, &a)?;
                }
                Ok(result)
            } else {
                Ok(a)
            }
        }
        fn atom(&mut self) -> Result<Polynomial> {
            if self.is("(") {
                self.at += 1;
                let value = self.expression()?;
                ensure!(self.is(")"), "unclosed polynomial group");
                self.at += 1;
                return Ok(value);
            }
            let token = self
                .tokens
                .get(self.at)
                .context("missing polynomial operand")?
                .clone();
            self.at += 1;
            if token.as_bytes()[0].is_ascii_digit() || token.starts_with('.') {
                let rational = Rational::parse(&token)?;
                Ok(if rational.0 == 0 {
                    BTreeMap::new()
                } else {
                    BTreeMap::from([(Vec::new(), rational)])
                })
            } else {
                ensure!(
                    token.as_bytes()[0].is_ascii_alphabetic() || token.starts_with('_'),
                    "invalid polynomial operand"
                );
                Ok(BTreeMap::from([(vec![token], Rational(1, 1))]))
            }
        }
    }
    let mut parser = Parse {
        tokens,
        at: 0,
        depth: 0,
    };
    let value = parser.expression()?;
    ensure!(
        parser.at == parser.tokens.len(),
        "expression outside polynomial verifier domain"
    );
    Ok(value)
}
pub fn exact_identity(left: &str, right: &str) -> Result<bool> {
    Ok(polynomial(left)? == polynomial(right)?)
}
pub fn check_execution(record: &Value, minimum: &str) -> Result<()> {
    let levels = [
        "written",
        "syntax_checked",
        "executed",
        "unit_tested",
        "model_inference",
        "training",
        "reproduced",
    ];
    let level = record["level"]
        .as_str()
        .and_then(|s| levels.iter().position(|x| *x == s))
        .context("unknown execution level")?;
    let expected = levels
        .iter()
        .position(|x| *x == minimum)
        .context("unknown minimum execution level")?;
    ensure!(
        level >= expected,
        "execution evidence does not establish {minimum}"
    );
    for key in [
        "command",
        "code_hash",
        "runtime",
        "dependencies",
        "inputs",
        "stdout",
        "limitations",
        "hardware",
        "seed",
    ] {
        ensure!(
            record.get(key).is_some(),
            "execution evidence missing {key}"
        );
    }
    Ok(())
}
pub fn check_execution_artifact(record: &Value, body: &str) -> Result<()> {
    let code = record["code"]
        .as_str()
        .context("execution evidence needs actual code")?;
    ensure!(
        !code.is_empty()
            && body.contains(code)
            && record["code_hash"] == crate::cache::hash(code.as_bytes()),
        "execution evidence does not bind displayed code"
    );
    if let Some(stdout) = record["stdout"].as_str() {
        ensure!(
            stdout.trim().is_empty() || body.contains(stdout.trim()),
            "observed execution output missing from chapter"
        );
    }
    Ok(())
}
pub fn check_example(record: &Value, body: &str) -> Result<()> {
    let kind = record["kind"]
        .as_str()
        .context("missing example authenticity")?;
    ensure!(
        [
            "conceptual_illustration",
            "synthetic_numerical",
            "deterministic_simulation",
            "executable_demonstration",
            "model_backed_execution",
            "experimental_reproduction",
            "source_reported_result"
        ]
        .contains(&kind),
        "unknown example authenticity"
    );
    let minimum = match kind {
        "deterministic_simulation" | "executable_demonstration" => Some("executed"),
        "model_backed_execution" => Some("model_inference"),
        "experimental_reproduction" => Some("reproduced"),
        _ => None,
    };
    if let Some(minimum) = minimum {
        check_execution(&record["execution"], minimum)?;
        check_execution_artifact(&record["execution"], body)?;
    }
    if ["model_backed_execution", "experimental_reproduction"].contains(&kind) {
        ensure!(
            record["state_origin"] == "model_execution",
            "hand specified vectors are not executed model states"
        );
        for key in ["model_revision", "tokenizer_revision", "attention_backend"] {
            ensure!(
                record["execution"][key]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()),
                "model execution missing {key}"
            );
        }
    }
    Ok(())
}
pub fn check_implementation(record: &Value, body: &str) -> Result<()> {
    let level = record["level"]
        .as_str()
        .context("missing implementation level")?;
    ensure!(
        [
            "component_snippet",
            "illustrative_demonstration",
            "end_to_end",
            "reproducible_experiment",
            "production_system"
        ]
        .contains(&level),
        "unknown implementation path level"
    );
    let complete = ["end_to_end", "reproducible_experiment", "production_system"].contains(&level);
    if record["claims_full_training_step"] == true {
        ensure!(complete, "complete training step requires end-to-end scope");
        for key in [
            "input_preparation",
            "forward",
            "hidden_state_extraction",
            "labels",
            "loss_terms",
            "reductions",
            "gradient_flow",
            "optimizer_update",
            "diagnostics",
            "tests",
        ] {
            let code = record["stages"][key]
                .as_str()
                .context("component snippet does not implement a complete training step")?;
            ensure!(
                !code.is_empty() && body.contains(code),
                "training stage has no code in chapter"
            );
        }
        check_execution(&record["execution"], "training")?;
    }
    if complete {
        ensure!(
            record.get("backend").is_some() && record.get("assumptions").is_some(),
            "complete implementation needs backend and assumptions"
        );
        check_execution(&record["execution"], "executed")?;
        check_execution_artifact(&record["execution"], body)?;
    }
    Ok(())
}
pub fn check_claim(record: &Value, policy: &str) -> Result<()> {
    let text = record["text"].as_str().context("claim needs exact text")?;
    ensure!(!text.trim().is_empty(), "empty claim");
    let class = record["classification"]
        .as_str()
        .context("claim needs classification")?;
    ensure!(
        [
            "definition",
            "identity",
            "proved_proposition",
            "empirical_observation",
            "reported_source_claim",
            "engineering_inference",
            "hypothesis",
            "speculation"
        ]
        .contains(&class),
        "unknown claim classification"
    );
    ensure!(
        record["contradictions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "claim has unresolved contradictory evidence"
    );
    ensure!(
        [
            "reported",
            "derived",
            "reproduced",
            "inferred",
            "hypothetical"
        ]
        .contains(&record["origin"].as_str().unwrap_or("")),
        "claim provenance remains unverified"
    );
    if text.split(|c: char| !c.is_alphabetic()).any(|s| {
        [
            "always",
            "never",
            "guarantee",
            "guarantees",
            "prove",
            "proves",
            "solve",
            "solves",
        ]
        .contains(&s.to_lowercase().as_str())
    }) {
        ensure!(
            record.get("scope_review").is_some() && record.get("limits").is_some(),
            "strong wording requires evidence-scope review"
        );
    }
    if ["reported_source_claim", "empirical_observation"].contains(&class)
        || record["origin"] == "reported"
    {
        let source = &record["source"];
        for key in [
            "id",
            "version",
            "location",
            "verification_method",
            "status",
            "limits",
            "snapshot",
            "excerpt",
        ] {
            ensure!(
                source.get(key).is_some_and(|v| !v.is_null() && v != ""),
                "claim provenance missing {key}"
            );
        }
        ensure!(
            source["status"] == "verified",
            "claim source remains unverified"
        );
        ensure!(
            source["contradictions"]
                .as_array()
                .is_none_or(Vec::is_empty),
            "claim source has unresolved contradictions"
        );
        if policy == "strict" {
            ensure!(
                source["origin"] == "supplied",
                "source-only policy rejects external evidence"
            );
        }
    }
    if class == "empirical_observation" {
        let e = &record["empirical"];
        for key in [
            "dataset",
            "model",
            "adaptation",
            "metric",
            "protocol",
            "hyperparameters",
            "model_selection",
            "runs",
            "uncertainty",
            "compute",
            "location",
            "value",
            "units",
        ] {
            ensure!(e.get(key).is_some(), "empirical result missing {key}");
        }
        ensure!(
            e["value"].as_f64().is_some_and(f64::is_finite),
            "empirical result requires finite measured value"
        );
        let u = &e["uncertainty"];
        let kind = u["type"].as_str().context("uncertainty needs type")?;
        ensure!(
            ["SD", "SE", "CI", "not_reported"].contains(&kind),
            "uncertainty must distinguish SD, SE, CI, not reported"
        );
        if kind != "not_reported" {
            ensure!(
                e["runs"].as_u64().is_some_and(|n| n >= 2),
                "repeated-run uncertainty requires actual run count"
            );
            ensure!(
                u["value"]
                    .as_f64()
                    .is_some_and(|x| x.is_finite() && x >= 0.),
                "uncertainty needs finite nonnegative value"
            );
        }
        if kind == "CI" {
            ensure!(
                u["level"].as_f64().is_some_and(|x| x > 0. && x < 1.),
                "confidence interval needs level"
            );
        }
        if e["comparison"] == true {
            ensure!(
                ["percentage_points", "relative_percent", "absolute"]
                    .contains(&e["change_unit"].as_str().unwrap_or("")),
                "metric change needs explicit percent convention"
            );
        }
    }
    Ok(())
}
pub fn check_failure(record: &Value) -> Result<()> {
    for key in [
        "symptom",
        "latent_property",
        "candidate_cause",
        "diagnostic",
        "measurement",
        "evidence_status",
        "remediation",
    ] {
        ensure!(
            record.get(key).is_some_and(|v| !v.is_null() && v != ""),
            "failure diagnosis missing {key}"
        );
    }
    ensure!(
        record["cause_established"] != true
            || record["evidence_status"] == "controlled_measurement",
        "symptom alone cannot establish cause"
    );
    if record["family"] == "representation" {
        ensure!(
            [
                "hidden_state_collapse",
                "selected_embedding_collapse",
                "directional_collapse",
                "dimensional_collapse",
                "generation_failure"
            ]
            .contains(&record["latent_property"].as_str().unwrap_or("")),
            "representation failure needs specific latent property"
        );
    }
    Ok(())
}
pub fn check_cost(record: &Value) -> Result<()> {
    for key in [
        "dense_tensor_size",
        "operations",
        "activation_memory",
        "backward_cost",
        "padding",
        "kernel",
        "hardware",
        "protocol",
        "kind",
    ] {
        ensure!(record.get(key).is_some(), "resource analysis missing {key}");
    }
    ensure!(
        ["theoretical_estimate", "measured_benchmark"]
            .contains(&record["kind"].as_str().unwrap_or("")),
        "resource analysis must distinguish estimates and benchmarks"
    );
    if record["kind"] == "measured_benchmark" {
        check_execution(&record["execution"], "executed")?;
    }
    if record["comparison"] == true {
        ensure!(
            ["equal_epochs", "equal_compute"].contains(&record["protocol"].as_str().unwrap_or("")),
            "cost comparison requires equal-epochs or equal-compute protocol"
        );
    }
    Ok(())
}
pub fn representation_metrics(states: &[Vec<f64>]) -> Result<Value> {
    ensure!(
        (2..=64).contains(&states.len()) && !states[0].is_empty() && states[0].len() <= 512,
        "diagnostics need 2-64 bounded observations"
    );
    let n = states.len();
    let width = states[0].len();
    ensure!(
        states
            .iter()
            .all(|r| r.len() == width && r.iter().all(|x| x.is_finite())),
        "invalid representation measurement dimensions or values"
    );
    let mean: Vec<_> = (0..width)
        .map(|j| states.iter().map(|r| r[j]).sum::<f64>() / n as f64)
        .collect();
    let centered: Vec<Vec<_>> = states
        .iter()
        .map(|r| r.iter().zip(&mean).map(|(x, m)| x - m).collect())
        .collect();
    let mut gram: Vec<Vec<f64>> = centered
        .iter()
        .map(|a| {
            centered
                .iter()
                .map(|b| a.iter().zip(b).map(|(a, b)| a * b).sum())
                .collect()
        })
        .collect();
    let trace: f64 = (0..n).map(|i| gram[i][i]).sum();
    let squares: f64 = gram.iter().flatten().map(|x| x * x).sum();
    ensure!(
        trace.is_finite() && squares.is_finite(),
        "representation measurement overflow"
    );
    for _ in 0..50 * n * n {
        let mut pair = (0, 1);
        for i in 0..n {
            for j in i + 1..n {
                if gram[i][j].abs() > gram[pair.0][pair.1].abs() {
                    pair = (i, j);
                }
            }
        }
        let (p, q) = pair;
        if gram[p][q].abs() <= 1e-12 * trace.max(1.) {
            break;
        }
        let theta = 0.5 * (2. * gram[p][q]).atan2(gram[q][q] - gram[p][p]);
        let (c, s) = (theta.cos(), theta.sin());
        let (app, aqq, apq) = (gram[p][p], gram[q][q], gram[p][q]);
        for i in 0..n {
            if i != p && i != q {
                let (aip, aiq) = (gram[i][p], gram[i][q]);
                gram[i][p] = c * aip - s * aiq;
                gram[p][i] = gram[i][p];
                gram[i][q] = s * aip + c * aiq;
                gram[q][i] = gram[i][q];
            }
        }
        gram[p][p] = c * c * app - 2. * s * c * apq + s * s * aqq;
        gram[q][q] = s * s * app + 2. * s * c * apq + c * c * aqq;
        gram[p][q] = 0.;
        gram[q][p] = 0.;
    }
    let mut eigen: Vec<_> = (0..n).map(|i| gram[i][i].max(0.)).collect();
    eigen.sort_by(|a, b| b.total_cmp(a));
    let probabilities: Vec<_> = eigen
        .iter()
        .map(|x| x / trace)
        .filter(|x| *x > 0. && x.is_finite())
        .collect();
    let effective = if probabilities.is_empty() {
        0.
    } else {
        (-probabilities.iter().map(|p| p * p.ln()).sum::<f64>()).exp()
    };
    Ok(
        json!({"norms":states.iter().map(|r|r.iter().map(|x|x*x).sum::<f64>().sqrt()).collect::<Vec<_>>(),"between_example_variance":trace/n as f64,"participation_ratio":if squares>0.{trace*trace/squares}else{0.},"singular_spectrum":eigen.iter().map(|x|x.sqrt()).collect::<Vec<_>>(),"effective_dimension":effective,"scope":"finite centered observations; singular spectrum/effective dimension do not establish causal utility"}),
    )
}
pub fn heldout_linear_probe(
    train: &[Vec<f64>],
    targets: &[f64],
    test: &[Vec<f64>],
    labels: &[f64],
    ridge: f64,
) -> Result<Value> {
    ensure!(
        !train.is_empty()
            && !test.is_empty()
            && train.len() == targets.len()
            && test.len() == labels.len()
            && ridge.is_finite()
            && ridge > 0.,
        "linear probe needs positive ridge and explicit held-out observations"
    );
    let width = train[0].len();
    ensure!(
        (1..=64).contains(&width)
            && train
                .iter()
                .chain(test)
                .all(|r| r.len() == width && r.iter().all(|x| x.is_finite()))
            && targets.iter().chain(labels).all(|x| x.is_finite()),
        "invalid linear probe dimensions or values"
    );
    let mut matrix = vec![vec![0.; width + 1]; width];
    for i in 0..width {
        for j in 0..width {
            matrix[i][j] =
                train.iter().map(|r| r[i] * r[j]).sum::<f64>() + if i == j { ridge } else { 0. };
        }
        matrix[i][width] = train.iter().zip(targets).map(|(r, y)| r[i] * y).sum();
    }
    for i in 0..width {
        let pivot = (i..width)
            .max_by(|a, b| matrix[*a][i].abs().total_cmp(&matrix[*b][i].abs()))
            .unwrap();
        matrix.swap(i, pivot);
        let divisor = matrix[i][i];
        ensure!(divisor.abs() >= 1e-15, "ill-conditioned linear probe");
        for v in &mut matrix[i] {
            *v /= divisor;
        }
        let row = matrix[i].clone();
        for (j, current) in matrix.iter_mut().enumerate() {
            if i != j {
                let factor = current[i];
                for (v, pivot) in current.iter_mut().zip(&row) {
                    *v -= factor * pivot;
                }
            }
        }
    }
    let weights: Vec<_> = matrix.iter().map(|r| r[width]).collect();
    let predicted: Vec<_> = test
        .iter()
        .map(|r| r.iter().zip(&weights).map(|(x, w)| x * w).sum::<f64>())
        .collect();
    let mse = predicted
        .iter()
        .zip(labels)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        / labels.len() as f64;
    ensure!(mse.is_finite(), "linear probe numerical overflow");
    Ok(
        json!({"heldout_mse":mse,"weights":weights,"predictions":predicted,"train_observations":train.len(),"heldout_observations":test.len(),"limits":"Supplied split and targets; predictive association is not causal usefulness."}),
    )
}
pub fn check_diagnostics(record: &Value) -> Result<()> {
    for key in ["measurement", "controls", "evidence", "limits"] {
        ensure!(
            record.get(key).is_some_and(|v| !v.is_null() && v != ""),
            "latent claim missing {key}"
        );
    }
    ensure!(
        !(record["measurement"] == "2d_projection"
            && ["intrinsic_rank", "causal_usefulness"]
                .contains(&record["conclusion"].as_str().unwrap_or(""))),
        "2D projection cannot establish intrinsic rank or causal usefulness"
    );
    if let Some(states) = record.get("states") {
        let states: Vec<Vec<f64>> = serde_json::from_value(states.clone())?;
        let measured = representation_metrics(&states)?;
        if let Some(expected) = record["expected"].as_object() {
            for (k, e) in expected {
                let a = &measured[k];
                let pairs = if let (Some(a), Some(e)) = (a.as_array(), e.as_array()) {
                    ensure!(a.len() == e.len(), "diagnostic length mismatch");
                    a.iter()
                        .zip(e)
                        .map(|(a, b)| (a.as_f64(), b.as_f64()))
                        .collect::<Vec<_>>()
                } else {
                    vec![(a.as_f64(), e.as_f64())]
                };
                for (a, b) in pairs {
                    let (a, b) = (
                        a.context("diagnostic is not numeric")?,
                        b.context("expected diagnostic not numeric")?,
                    );
                    ensure!(
                        (a - b).abs() <= 1e-12 + 1e-9 * a.abs().max(b.abs()),
                        "representation diagnostic disagrees with computed {k}"
                    );
                }
            }
        }
    }
    if let Some(probe) = record.get("linear_probe") {
        let train = serde_json::from_value::<Vec<Vec<f64>>>(probe["train"].clone())?;
        let test = serde_json::from_value::<Vec<Vec<f64>>>(probe["test"].clone())?;
        let targets = serde_json::from_value::<Vec<f64>>(probe["targets"].clone())?;
        let labels = serde_json::from_value::<Vec<f64>>(probe["labels"].clone())?;
        let measured = heldout_linear_probe(
            &train,
            &targets,
            &test,
            &labels,
            probe["ridge"].as_f64().unwrap_or(1e-6),
        )?;
        if let Some(expected) = record["expected_mse"].as_f64() {
            let actual = measured["heldout_mse"].as_f64().unwrap();
            ensure!(
                (expected - actual).abs() <= 1e-10 + 1e-6 * expected.abs().max(actual.abs()),
                "held-out measurement disagrees with observations"
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_depth_preserves_explicit_and_source_limits() {
        let c = json!({"objectives":["Prove the identity","Trace algorithm state"],"depth":{"visual":"working"},"signals":{"source_gap":true}});
        let p = allocate_depth(&c, "strict").unwrap();
        assert_eq!(p["levels"]["mathematical"], 5);
        assert_eq!(p["levels"]["algorithmic"], 3);
        assert_eq!(p["levels"]["visual"], 2);
        assert_eq!(p["external_retrieval_permitted"], false);
        assert!(!p["unresolved"].as_array().unwrap().is_empty());
    }
    #[test]
    fn numerical_refutes_and_does_not_execute_code() {
        assert!(
            check_numerical(&json!({"expression":"2*x","variables":{"x":3},"expected":6}))
                .is_empty()
        );
        assert!(
            !check_numerical(&json!({"expression":"2*x","variables":{"x":3},"expected":7}))
                .is_empty()
        );
        assert!(arithmetic("__import__('os')", &json!({})).is_err());
    }
    #[test]
    fn capability_dependencies_are_native() {
        let ids = resolve_capabilities(&["S04".into()]).unwrap();
        assert!(ids.contains(&"S04".into()));
        assert!(resolve_capabilities(&["bad".into()]).is_err());
    }
    #[test]
    fn exact_polynomial_identities_do_not_use_samples() {
        assert!(exact_identity("(x+y)**2", "x*x+2*x*y+y*y").unwrap());
        assert!(exact_identity("0.1*x+0.2*x", "3*x/10").unwrap());
        assert!(!exact_identity("x*x", "x*x+1").unwrap());
        assert!(exact_identity("sin(x)", "x").is_err());
        assert!(exact_identity("x**99", "x").is_err());
    }
    #[test]
    fn representation_and_probe_measurements_are_computed() {
        let measured = representation_metrics(&[vec![1., 0.], vec![-1., 0.]]).unwrap();
        assert_eq!(measured["between_example_variance"], 1.);
        assert_eq!(measured["participation_ratio"], 1.);
        assert!((measured["effective_dimension"].as_f64().unwrap() - 1.).abs() < 1e-12);
        let probe =
            heldout_linear_probe(&[vec![1.], vec![2.]], &[2., 4.], &[vec![3.]], &[6.], 1e-6)
                .unwrap();
        assert!(probe["heldout_mse"].as_f64().unwrap() < 1e-10);
        assert!(representation_metrics(&[vec![1.], vec![f64::NAN]]).is_err());
    }
    #[test]
    fn code_evidence_binds_displayed_artifacts() {
        let code = "let x = 1;";
        let evidence =
            json!({"code":code,"code_hash":crate::cache::hash(code.as_bytes()),"stdout":"1"});
        assert!(check_execution_artifact(&evidence, "let x = 1;\nOutput: 1").is_ok());
        assert!(check_execution_artifact(&evidence, "let x = 2;\nOutput: 1").is_err());
    }
}
