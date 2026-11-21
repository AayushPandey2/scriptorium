use serde_json::{Value,json};
use std::{fs,path::Path,process::Command};
fn run(args:&[&str])->Value{
    let output=Command::new(env!("CARGO_BIN_EXE_scribe")).args(args).env("PATH","").output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));serde_json::from_slice(&output.stdout).unwrap()
}
fn save(path:&Path,value:&Value){fs::write(path,serde_json::to_vec_pretty(value).unwrap()).unwrap();}
#[test]
fn approved_authored_note_is_published_with_python_absent_from_path(){
    let dir=tempfile::tempdir().unwrap();let vault=dir.path().to_str().unwrap();
    let start=run(&["notes","start","formula sheet for mechanics","--vault",vault]);
    let sid=start["state"]["request_id"].as_str().unwrap();
    let response=run(&["notes","respond","--vault",vault,"--session-id",sid,"--reply","use your recommendations"]);
    assert_eq!(response["ready_to_generate"],true);let node=&response["state"]["collection_plan"]["nodes"][0];let cid=node["concept_id"].as_str().unwrap();
    let body="# Mechanics\n\n## Equations\n- $F=ma$: net force equals mass times acceleration for constant mass in an inertial frame.\n";
    let drafts=dir.path().join("drafts.json");save(&drafts,&json!({cid:body}));
    let review=dir.path().join("reviews.json");save(&review,&json!({"verifier":"fixture reviewer","method":"Inspect supplied formula assumptions and exact body","limitations":"classical constant-mass mechanics fixture","result":"pass","evidence":[{"chapter":cid,"hash":scriptorium_core::cache::hash(body.as_bytes()),"excerpt":"constant mass in an inertial frame"}]}));
    let result=run(&["notes","generate","--vault",vault,"--session-id",sid,"--drafts",drafts.to_str().unwrap(),"--reviews",review.to_str().unwrap()]);
    assert_eq!(result["complete"],true);let path=dir.path().join(node["proposed_path"].as_str().unwrap());let note=fs::read_to_string(path).unwrap();assert!(note.starts_with("---\n"));assert!(note.contains("topic/"));assert!(note.contains(body.trim()));
    let properties=run(&["properties","--topic","Mechanics"]);assert_eq!(properties["topic"],"Mechanics");
    let indexed=run(&["vault","--action","index","--vault",vault]);assert_eq!(indexed.as_array().unwrap().len(),1);
}
