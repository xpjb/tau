use super::*;
fn item(id: &str) -> Value { json!({"type":"reasoning","id":id,"encrypted_content":"private-cipher","summary":[]}) }
fn selected() -> SessionModel { SessionModel {provider:"openai-codex".into(),model_id:"gpt-6-astra".into()} }
#[test]
fn only_complete_encrypted_reasoning_is_recoverable() {
    let valid = item("rs_good"); assert_eq!(item_id(&valid),Some("rs_good"));
    for (key,value) in [("type",json!("function_call")),("status",json!("in_progress")),("id",json!("")),
        ("encrypted_content",json!(" ")), ("summary",Value::Null), ("summary",json!([{"type":"text","text":"x"}]))] {
        let mut invalid = valid.clone(); invalid[key] = value; assert_eq!(item_id(&invalid),None,"{key}");
    }
    let capture=Recovery::default(); Replay::new(&selected(),"account","https://fixture/codex",&[]).capture_into(&capture);
    capture.record(&valid); capture.record(&valid);
    assert_eq!(capture.snapshot().unwrap()["items"].as_array().unwrap().len(),1);
    assert!(capture.retry_safe()); capture.image_started(); assert!(!capture.retry_safe());
}
#[test]
fn scope_dedup_and_rejection_follow_only_the_selected_branch() {
    let capture=Recovery::default(); Replay::new(&selected(),"account","https://fixture/codex/",&[]).capture_into(&capture);
    capture.record(&item("rs_one")); capture.record(&item("rs_two"));
    let message=json!({"role":"reasoning_recovery","recovery":capture.snapshot().unwrap()});
    for (account,endpoint) in [("other","https://fixture/codex"),("account","https://other/codex")] {
        assert!(Replay::new(&selected(),account,endpoint,&[]).items(&message).is_empty());
    }
    for model in [SessionModel {provider:"other".into(),..selected()},SessionModel {model_id:"other".into(),..selected()}] {
        assert!(Replay::new(&model,"account","https://fixture/codex",&[]).items(&message).is_empty());
    }
    let mut foreign_api=message.clone(); foreign_api["recovery"]["scope"]["api"]=json!("other");
    assert!(Replay::new(&selected(),"account","https://fixture/codex",&[]).items(&foreign_api).is_empty());
    let mut replay=Replay::new(&selected(),"account","https://fixture/codex",&[]);
    assert_eq!(replay.items(&message).len(),2); assert!(replay.items(&message).is_empty());
    let rejected=Recovery::default(); replay.capture_into(&rejected); rejected.reject(&["rs_one".into()]);
    let marker=json!({"role":"reasoning_recovery","recovery":rejected.snapshot().unwrap()});
    let completed=json!({"role":"assistant","codex_output":[item("rs_two")]});
    assert!(Replay::new(&selected(),"account","https://fixture/codex",&[marker.clone(),completed]).items(&message).is_empty());
    // A fork before the rejection retains the eligible original checkpoint.
    assert_eq!(Replay::new(&selected(),"account","https://fixture/codex",&[]).items(&message).len(),2);
    let mut next=Replay::new(&selected(),"account","https://fixture/codex",&[marker]);
    assert_eq!(next.items(&message),vec![item("rs_two")]);
    let again=Recovery::default(); next.capture_into(&again); again.record(&item("rs_two"));
    assert!(!again.advanced(),"Re-emitting an input checkpoint is not new progress");
}
