use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use scriptorium_core::{
    assets::{AssetRecord, AssetStore, Provenance},
    config::Config,
    network::{self, DownloadPolicy},
    planning::{self, KnowledgePlan, VisualState},
    rendering, research,
    validation::{self, NoteDraft, VisualReceipt},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    name = "scribe",
    version,
    about = "Rust local visual planning, research retrieval and verified publication"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Setfolder {
        folder: PathBuf,
        #[arg(long)]
        configuration: Option<PathBuf>,
    },
    ResolveVault {
        #[arg(long)]
        vault: Option<PathBuf>,
        #[arg(long)]
        configuration: Option<PathBuf>,
    },
    Audit {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    ValidateRegistry {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    Route {
        request: String,
        #[arg(long)]
        source_policy: Option<String>,
        #[arg(long)]
        has_source: bool,
        #[arg(long)]
        trace: bool,
    },
    Properties {
        #[arg(long)]
        topic: String,
        #[arg(long, default_value = "deep-notes")]
        note_type: String,
        #[arg(long)]
        domain: Option<String>,
        #[arg(long)]
        master_topic: Option<String>,
    },
    Engine {
        action: String,
        #[arg(long)]
        input: PathBuf,
    },
    Vault {
        #[arg(long, value_parser=["index","graph","patch","rename"])]
        action: String,
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        topic: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        expected_hash: Option<String>,
        #[arg(long)]
        apply: bool,
    },
    Notes {
        #[command(subcommand)]
        action: Notes,
    },
    Assets {
        #[command(subcommand)]
        action: Assets,
    },
    Research {
        #[arg(long, required = true)]
        url: Vec<String>,
        #[arg(long)]
        cache: PathBuf,
    },
    Renderers,
    Render {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Config {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        note: PathBuf,
    },
    Validate {
        file: PathBuf,
    },
}
#[derive(Subcommand)]
enum Notes {
    Generate {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        drafts: PathBuf,
        #[arg(long)]
        evidence: Option<PathBuf>,
        #[arg(long)]
        reviews: PathBuf,
        #[arg(long)]
        expected_hashes: Option<PathBuf>,
    },
    Compose {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        drafts: Option<PathBuf>,
        #[arg(long)]
        evidence: Option<PathBuf>,
        #[arg(long)]
        reviews: Option<PathBuf>,
        #[arg(long)]
        vault: Option<PathBuf>,
        #[arg(long)]
        prepare: bool,
        #[arg(long)]
        validate: bool,
        #[arg(long)]
        publish: bool,
        #[arg(long)]
        expected_hashes: Option<PathBuf>,
    },
    Start {
        #[arg(default_value = "/scribe-notes")]
        request: String,
        #[arg(long)]
        vault: Option<PathBuf>,
        #[arg(long)]
        has_source: bool,
        #[arg(long)]
        source_section: Vec<String>,
    },
    Respond {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        reply: String,
    },
    Update {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        receipts: PathBuf,
        #[arg(long)]
        drafts: PathBuf,
        #[arg(long)]
        expected_hashes: PathBuf,
    },
    Plan {
        #[arg(long)]
        spec: PathBuf,
    },
    Audit {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        receipts: PathBuf,
    },
    Publish {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        receipts: PathBuf,
        #[arg(long)]
        drafts: PathBuf,
    },
}
#[derive(Subcommand)]
enum Assets {
    Render {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        note: PathBuf,
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        visual_id: Option<String>,
    },
    Fetch {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        note: PathBuf,
        #[arg(long)]
        url: String,
        #[arg(long)]
        provenance: PathBuf,
        #[arg(long)]
        visual_id: Option<String>,
    },
    Import {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        note: PathBuf,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        mime: String,
        #[arg(long)]
        provenance: PathBuf,
        #[arg(long)]
        visual_id: Option<String>,
    },
    Verify {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        note: PathBuf,
        #[arg(long)]
        record: PathBuf,
    },
    Embed {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        note: PathBuf,
        #[arg(long)]
        record: PathBuf,
        #[arg(long)]
        alt: String,
    },
}
fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}
fn global_configuration(path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = path {
        return Ok(path);
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .context("home directory unavailable; pass --configuration")?;
    Ok(PathBuf::from(home).join(".scriptorium-config.json"))
}
fn output(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
fn store(vault: &Path, note: &Path, config: &Config) -> Result<AssetStore> {
    AssetStore::new(vault, config.attachment_dir(vault, note)?)
}
fn asset_output(record: AssetRecord, visual_id: Option<String>) -> Result<()> {
    if let Some(id) = visual_id {
        ensure!(!id.trim().is_empty(), "visual ID must not be empty");
        output(&VisualReceipt {
            visual_id: id,
            local_path: record.local_path.clone(),
            sha256: record.sha256.clone(),
            provenance: serde_json::to_value(&record.provenance)?,
            state: VisualState::Verified,
        })
    } else {
        output(&record)
    }
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        let _ = writeln!(
            io::stderr(),
            "{}",
            serde_json::json!({"error":format!("{e:#}")})
        );
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;
    match cli.command {
        Command::Setfolder {
            folder,
            configuration,
        } => output(&scriptorium_core::vault::set_notes_folder(
            &folder,
            &global_configuration(configuration)?,
        )?)?,
        Command::ResolveVault {
            vault,
            configuration,
        } => output(&scriptorium_core::vault::resolve_vault(
            vault.as_deref(),
            &global_configuration(configuration)?,
            &std::env::current_dir()?,
        )?)?,
        Command::Audit { root } => output(&scriptorium_core::quality::audit_repository(&root)?)?,
        Command::ValidateRegistry { root } => {
            output(&scriptorium_core::quality::validate_registry(&root)?)?
        }
        Command::Route {
            request,
            source_policy,
            has_source,
            trace,
        } => output(&scriptorium_core::orchestration::route_request(
            &request,
            &scriptorium_core::orchestration::RouteOptions {
                source_policy,
                has_source,
                trace,
            },
        )?)?,
        Command::Properties {
            topic,
            note_type,
            domain,
            master_topic,
        } => output(&scriptorium_core::metadata::build_properties(
            &topic,
            &note_type,
            domain.as_deref(),
            master_topic.as_deref(),
        )?)?,
        Command::Engine { action, input } => {
            let spec: serde_json::Value = read(&input)?;
            use scriptorium_core::{evidence as e, knowledge as k, optimization as o};
            let result = match action.as_str() {
                "concepts" => k::registry_action(&spec)?,
                "coverage" => k::evaluate_coverage_ledger(&spec)?,
                "claims" => k::audit_claim_traceability(&spec)?,
                "impact" => k::compute_change_impact(&spec)?,
                "consistency" => k::check_multi_artifact_consistency(&spec)?,
                "research-mode" => e::resolve_research_mode(&spec)?,
                "research-plan" => e::normalize_research_plan(&spec)?,
                "research-questions" => e::plan_research(&spec)?,
                "source-quality" => e::evaluate_source(&spec)?,
                "evidence" => e::normalize_evidence_registry(&spec)?,
                "research-coverage" => e::analyze_research_coverage(&spec)?,
                "research-package" => e::research_package(&spec)?,
                "cache-key" => o::dependency_cache_key(&spec)?,
                "partition-context" => o::partition_context(&spec)?,
                "compile-context" => o::compile_context(&spec)?,
                "source-segments" => o::source_segments(&spec)?,
                "retrieve-passages" => o::retrieve_passages(&spec)?,
                "depth" => scriptorium_core::pedagogy::allocate_depth(
                    &spec,
                    spec["source_policy"].as_str().unwrap_or("adaptive"),
                )?,
                "numerical-check" => {
                    serde_json::json!({"errors":scriptorium_core::pedagogy::check_numerical(&spec)})
                }
                "exact-identity" => {
                    serde_json::json!({"equivalent":scriptorium_core::pedagogy::exact_identity(spec["lhs"].as_str().context("lhs required")?,spec["rhs"].as_str().context("rhs required")?)?})
                }
                "explanation" => scriptorium_core::pedagogy::explanation_blueprint(
                    spec["topic"].as_str().context("topic required")?,
                    spec["master"].as_str().unwrap_or("M1"),
                    spec["audience"].as_str().unwrap_or("intermediate"),
                    spec["source_policy"].as_str().unwrap_or("adaptive"),
                )?,
                "question" => {
                    serde_json::json!({"markdown":scriptorium_core::assessment::render_question(&spec)?})
                }
                "lifecycle-depth" => scriptorium_core::lifecycle::allocate_depth(&spec)?,
                "learning-paths" => scriptorium_core::lifecycle::learning_paths(&spec)?,
                "prerequisite-gaps" => scriptorium_core::lifecycle::prerequisite_gaps(&spec)?,
                "reconcile-notes" => scriptorium_core::lifecycle::reconcile_notes(&spec)?,
                "checkpoint" => scriptorium_core::lifecycle::run_checkpoint(&spec)?,
                "competency" => scriptorium_core::lifecycle::competency_coverage(&spec)?,
                "collection-impact" => {
                    scriptorium_core::lifecycle::collection_change_impact(&spec)?
                }
                "feedback" => scriptorium_core::lifecycle::reader_feedback(&spec)?,
                "uncertainty" => k::uncertainty_ledger(&spec)?,
                "resources" => e::curate_learning_resources(&spec)?,
                "source-freshness" => e::source_freshness(&spec)?,
                "research-stop" => e::should_stop_research(&spec)?,
                "notation"=>scriptorium_core::lifecycle::notation_consistency(&spec)?,
                "granularity"=>scriptorium_core::lifecycle::evaluate_granularity(&spec)?,
                "split-concept"=>scriptorium_core::lifecycle::apply_granularity_split(&spec)?,
                "synthesis"=>scriptorium_core::lifecycle::plan_synthesis(&spec)?,
                _ => bail!("unsupported engine action: {action}"),
            };
            output(&result)?;
        }
        Command::Vault {
            action,
            vault,
            topic,
            file,
            input,
            expected_hash,
            apply,
        } => {
            use scriptorium_core::vault as services;
            if action == "index" || action == "graph" {
                let notes = services::index(&vault, topic.as_deref())?;
                if action == "index" {
                    output(&notes)?;
                } else {
                    output(&services::graph(&notes))?;
                }
            } else {
                let relative = file.context("edit requires --file")?;
                let path = validation::safe_path(&vault, &relative, true)?;
                let original = fs::read_to_string(path)?;
                let spec: serde_json::Value = read(&input.context("edit requires --input JSON")?)?;
                let changed = if action == "patch" {
                    let changes = serde_json::from_value(
                        spec.get("properties")
                            .cloned()
                            .unwrap_or(serde_json::json!({})),
                    )?;
                    let tags: Vec<String> = serde_json::from_value(
                        spec.get("add_tags")
                            .cloned()
                            .unwrap_or(serde_json::json!([])),
                    )?;
                    services::patch_properties(&original, &changes, &tags)?
                } else {
                    services::rename_link(
                        &original,
                        spec["old"].as_str().context("old link missing")?,
                        spec["new"].as_str().context("new link missing")?,
                    )?
                };
                let applied = if apply {
                    services::write_checked(&vault, &relative, &changed, expected_hash.as_deref())?
                } else {
                    false
                };
                output(
                    &serde_json::json!({"changed":original!=changed,"applied":applied,"content":changed,"original_hash":scriptorium_core::cache::hash(original.as_bytes())}),
                )?;
            }
        }
        Command::Notes { action } => match action {
            Notes::Generate {
                vault,
                session_id,
                drafts,
                evidence,
                reviews,
                expected_hashes,
            } => {
                let mut session = scriptorium_core::sessions::load(&vault, &session_id)?;
                let evidence = if let Some(p) = evidence {
                    read(&p)?
                } else {
                    serde_json::json!({})
                };
                let hashes = if let Some(p) = expected_hashes {
                    read(&p)?
                } else {
                    std::collections::BTreeMap::new()
                };
                let report = scriptorium_core::authoring::generate_session(
                    &vault,
                    &session,
                    &read(&drafts)?,
                    &evidence,
                    &read(&reviews)?,
                    &hashes,
                )?;
                if report["complete"] == true {
                    session.stage = "COMPLETED".into();
                    scriptorium_core::sessions::save(&vault, &session)?;
                }
                output(&report)?;
                ensure!(
                    report["complete"] == true,
                    "generation blocked by contract obligations"
                );
            }
            Notes::Compose {
                spec,
                drafts,
                evidence,
                reviews,
                vault,
                prepare,
                validate,
                publish,
                expected_hashes,
            } => {
                use scriptorium_core::composition as c;
                ensure!(
                    [prepare, validate, publish].iter().filter(|b| **b).count() <= 1,
                    "choose one of prepare, validate or publish"
                );
                let plan = c::build_composition(&read(&spec)?)?;
                if let Some(path) = drafts {
                    let drafts: std::collections::BTreeMap<String, String> = read(&path)?;
                    let (prepared, receipts) = if let Some(root) = &vault {
                        c::prepare_visuals(root, &plan, &drafts).await?
                    } else {
                        ensure!(
                            plan["visuals"].as_array().is_none_or(Vec::is_empty),
                            "visual preparation requires --vault"
                        );
                        (drafts, Vec::new())
                    };
                    let prepared = c::prepare_drafts(&plan, &prepared)?;
                    if prepare {
                        output(
                            &serde_json::json!({"plan":plan,"drafts":prepared,"receipts":receipts}),
                        )?;
                    } else {
                        let evidence = if let Some(p) = evidence {
                            read(&p)?
                        } else {
                            serde_json::json!({})
                        };
                        let reviews = if let Some(p) = reviews {
                            read(&p)?
                        } else {
                            serde_json::json!({})
                        };
                        let report =
                            c::validate_composition(&plan, &prepared, &evidence, &reviews)?;
                        if publish {
                            ensure!(
                                report["complete"] == true,
                                "educational publication blocked: {}",
                                report["errors"]
                            );
                            let root = vault.context("publication requires --vault")?;
                            let visual_plan = c::to_visual_plan(&plan)?;
                            let docs = c::compose_documents(&plan, &prepared)?
                                .into_iter()
                                .map(|(path, content)| NoteDraft {
                                    path: path.into(),
                                    content,
                                })
                                .collect::<Vec<_>>();
                            let folder = PathBuf::from(
                                plan["folder"]
                                    .as_str()
                                    .context("composition folder missing")?,
                            );
                            if root.join(&folder).exists() {
                                let hashes = if let Some(path) = expected_hashes {
                                    read(&path)?
                                } else {
                                    std::collections::BTreeMap::new()
                                };
                                output(&scriptorium_core::updates::update_collection(
                                    &root,
                                    &folder,
                                    &visual_plan,
                                    &receipts,
                                    &docs,
                                    &hashes,
                                )?)?;
                            } else {
                                output(&validation::publish_collection(
                                    &root,
                                    &folder,
                                    &visual_plan,
                                    &receipts,
                                    &docs,
                                )?)?;
                            }
                        } else {
                            output(&report)?;
                            ensure!(
                                !validate || report["complete"] == true,
                                "educational validation failed"
                            );
                        }
                    }
                } else {
                    ensure!(
                        !prepare && !publish && !validate,
                        "selected action requires --drafts"
                    );
                    output(&plan)?;
                }
            }
            Notes::Start {
                request,
                vault,
                has_source,
                source_section,
            } => {
                let state = scriptorium_core::sessions::start_with_sections(
                    &request,
                    has_source,
                    &source_section,
                )?;
                let saved = if let Some(vault) = vault {
                    Some(scriptorium_core::sessions::save(&vault, &state)?)
                } else {
                    None
                };
                output(
                    &serde_json::json!({"state":state,"saved_path":saved,"prompt":scriptorium_core::sessions::prompt(&state)}),
                )?;
            }
            Notes::Respond {
                vault,
                session_id,
                reply,
            } => {
                let state = scriptorium_core::sessions::load(&vault, &session_id)?;
                let result = scriptorium_core::sessions::respond(state, &reply)?;
                scriptorium_core::sessions::save(&vault, &result.state)?;
                output(&result)?;
            }
            Notes::Update {
                vault,
                destination,
                plan,
                receipts,
                drafts,
                expected_hashes,
            } => output(&scriptorium_core::updates::update_collection(
                &vault,
                &destination,
                &read(&plan)?,
                &read::<Vec<VisualReceipt>>(&receipts)?,
                &read::<Vec<NoteDraft>>(&drafts)?,
                &read(&expected_hashes)?,
            )?)?,
            Notes::Plan { spec } => output(&planning::build_plan(read(&spec)?)?)?,
            Notes::Audit {
                vault,
                plan,
                receipts,
            } => {
                let report = validation::audit_collection(
                    &vault,
                    &read::<KnowledgePlan>(&plan)?,
                    &read::<Vec<VisualReceipt>>(&receipts)?,
                )?;
                output(&report)?;
                ensure!(
                    report.artifact_complete,
                    "collection has unfulfilled artifact obligations"
                );
            }
            Notes::Publish {
                vault,
                destination,
                plan,
                receipts,
                drafts,
            } => {
                output(&validation::publish_collection(
                    &vault,
                    &destination,
                    &read::<KnowledgePlan>(&plan)?,
                    &read::<Vec<VisualReceipt>>(&receipts)?,
                    &read::<Vec<NoteDraft>>(&drafts)?,
                )?)?;
            }
        },
        Command::Renderers => output(&rendering::families())?,
        Command::Render { spec, output: path } => {
            let artifact = rendering::render(&read(&spec)?)?;
            ensure!(
                path.extension().and_then(|p| p.to_str()) == Some(artifact.extension.as_str()),
                "output extension must match {}",
                artifact.extension
            );
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .context("create artifact; existing files are preserved")?;
            file.write_all(&artifact.bytes)?;
            file.sync_all()?;
            output(
                &serde_json::json!({"path":path,"mime":artifact.mime,"sha256":scriptorium_core::cache::hash(&artifact.bytes),"renderer":artifact.renderer}),
            )?;
        }
        Command::Config { vault, note } => output(
            &serde_json::json!({"config":config,"attachment_directory":config.attachment_dir(&vault,&note)?}),
        )?,
        Command::Validate { file } => {
            validation::validate_properties(&fs::read_to_string(file)?)?;
            output(
                &serde_json::json!({"valid_properties":true,"educational_review_required":true}),
            )?;
        }
        Command::Research { url, cache } => {
            let report = research::retrieve(url, config, cache).await?;
            output(&report)?;
            ensure!(report.complete, "one or more sources failed");
        }
        Command::Assets { action } => match action {
            Assets::Render {
                vault,
                note,
                spec,
                visual_id,
            } => {
                let spec: serde_json::Value = read(&spec)?;
                let objective = spec
                    .get("objective")
                    .and_then(|x| x.as_str())
                    .context("render specification needs objective")?;
                let artifact = rendering::render(&spec)?;
                if artifact.mime != "image/svg+xml" {
                    bail!(
                        "this backend produces a nonimage artifact; image asset publication requires SVG or a decoded raster"
                    );
                }
                asset_output(
                    store(&vault, &note, &config)?.store_generated(&artifact, objective)?,
                    visual_id,
                )?;
            }
            Assets::Import {
                vault,
                note,
                file,
                mime,
                provenance,
                visual_id,
            } => {
                ensure!(
                    fs::metadata(&file)?.len() <= scriptorium_core::assets::MAX_BYTES as u64,
                    "input exceeds image limit"
                );
                let record = store(&vault, &note, &config)?.store_bytes(
                    &fs::read(file)?,
                    &mime,
                    read(&provenance)?,
                )?;
                asset_output(record, visual_id)?;
            }
            Assets::Fetch {
                vault,
                note,
                url,
                provenance,
                visual_id,
            } => {
                let mut p: Provenance = read(&provenance)?;
                p.validate()?;
                ensure!(
                    p.image_url == url,
                    "provenance image URL must match requested URL"
                );
                let policy = DownloadPolicy {
                    max_bytes: config
                        .max_download_bytes
                        .min(scriptorium_core::assets::MAX_BYTES),
                    timeout_seconds: config.timeout_seconds,
                    max_redirects: 3,
                };
                let downloaded = network::fetch_image(&url, &policy).await?;
                p.retrieved_at = scriptorium_core::cache::now()?;
                p.final_url = Some(downloaded.final_url.clone());
                let record = store(&vault, &note, &config)?.store_bytes(
                    &downloaded.bytes,
                    &downloaded.mime,
                    p,
                )?;
                asset_output(record, visual_id)?;
            }
            Assets::Verify {
                vault,
                note,
                record,
            } => {
                store(&vault, &note, &config)?.verify(&read(&record)?)?;
                output(&serde_json::json!({"verified":true}))?;
            }
            Assets::Embed {
                vault,
                note,
                record,
                alt,
            } => {
                println!(
                    "{}",
                    store(&vault, &note, &config)?.embed(&read(&record)?, &alt)?
                );
            }
        },
    }
    Ok(())
}
