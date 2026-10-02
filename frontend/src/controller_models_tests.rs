use super::*;
use std::sync::Arc;

fn catalog(revision: u64, slug: &str) -> tau_protocol::ModelCatalog {
    tau_protocol::ModelCatalog { revision, default_model: Some(slug.parse().unwrap()), models: vec![
        SlashCommandArgument { value: slug.into(), description: Some("Cached suggestion".into()) },
    ], unresolved_providers: vec![] }
}

#[test]
fn model_choice_and_remembered_default_commit_together_or_neither_changes() {
    let root = tempfile::tempdir().unwrap();
    let mut c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    c.new_chat().unwrap();
    let id = c.account.selected.clone().unwrap();
    c.choose_model(&id, "fixture/first").unwrap();
    c.draft("Keep draft".into()).unwrap();
    let db = rusqlite::Connection::open(root.path().join("client.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_choice BEFORE INSERT ON local WHEN NEW.key='account' BEGIN SELECT RAISE(ABORT,'full');END").unwrap();
    assert!(c.choose_model(&id, "fixture/second").is_err());
    let first: SessionModel = "fixture/first".parse().unwrap();
    assert_eq!(c.selected_model(&id), Some(&first));
    assert_eq!(c.account.last_model.as_ref(), Some(&first));
    assert_eq!(c.store.load_chat(&c.identity, &id).unwrap().model_choice, Some(first.clone()));
    assert_eq!(c.store.get::<Account>(&c.identity, "account").unwrap().last_model, Some(first));
    assert_eq!(c.selected().unwrap().local.draft, "Keep draft");
    assert!(c.selected().unwrap().local.pending.is_empty());
    assert!(c.requests.is_empty(), "No selection RPC even when local persistence fails");
}

#[test]
fn model_catalog_is_account_cached_restartable_ordered_and_source_fenced() {
    let root = tempfile::tempdir().unwrap();
    let mut c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    c.network_event(transport::Event::Source(1, "source-a".into())).unwrap();
    c.message(ServerMessage::ModelCatalog { catalog: catalog(20, "fixture/new") }).unwrap();
    c.message(ServerMessage::ModelCatalog { catalog: catalog(3, "fixture/late") }).unwrap();
    assert_eq!(c.model_catalog.models[0].value, "fixture/new");
    drop(c);
    let mut c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    assert!(c.account.selected.is_none());
    assert_eq!(c.model_catalog.models[0].value, "fixture/new", "Suggestions exist before any chat or network");
    assert!(c.store.get::<tau_protocol::ModelCatalog>("another-account", "model-catalog").unwrap().models.is_empty());
    // Revisions are daemon/connection scoped, not permanent freshness stamps.
    c.message(ServerMessage::ModelCatalog { catalog: catalog(1, "fixture/restarted") }).unwrap();
    c.new_chat().unwrap();
    let id = c.account.selected.clone().unwrap();
    assert_eq!(c.selected_model(&id), Some(&"fixture/restarted".parse().unwrap()));
    c.choose_model(&id, "fixture/manual-missing-from-catalog").unwrap();
    c.message(ServerMessage::ModelCatalog { catalog: catalog(2, "fixture/refreshed") }).unwrap();
    assert_eq!(c.selected_model(&id), Some(&"fixture/manual-missing-from-catalog".parse().unwrap()));
    c.network_event(transport::Event::Source(2, "source-b".into())).unwrap();
    assert!(c.model_catalog.models.is_empty());
    assert!(c.store.get::<tau_protocol::ModelCatalog>(&c.identity, "model-catalog").unwrap().models.is_empty());
    assert!(c.account.create_blocked, "Source changes still fence automatic work");
    assert_eq!(c.selected_model(&id), Some(&"fixture/manual-missing-from-catalog".parse().unwrap()), "Authored choices are not disposable metadata");
}

#[test]
fn attachment_preparation_and_alias_keep_the_exact_pinned_model() {
    let root = tempfile::tempdir().unwrap();
    let mut c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    c.new_chat().unwrap();
    let id = c.account.selected.clone().unwrap();
    c.choose_model(&id, "fixture/exact").unwrap();
    let file = root.path().join("input.txt"); std::fs::write(&file, "attachment").unwrap();
    c.attach(&file, None).unwrap(); c.draft("With file".into()).unwrap(); c.send_prompt().unwrap();
    let request = c.selected().unwrap().local.pending[0].request.id.clone();
    c.finish_create(&id, "confirmed").unwrap();
    assert_eq!(c.selected_model("confirmed"), Some(&"fixture/exact".parse().unwrap()));
    let pending = &c.selected().unwrap().local.pending[0];
    assert_eq!(pending.request.id, request);
    assert!(matches!(&pending.request.command, ClientCommand::Prompt {model:Some(model),session_id,..}
        if model.model_id == "exact" && session_id == "confirmed"));
    assert!(pending.files[0].path.exists());
}

#[test]
fn cold_catalog_snapshots_keep_last_good_hints_until_each_provider_is_ready() {
    let root = tempfile::tempdir().unwrap();
    let mut c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    c.message(ServerMessage::ModelCatalog { catalog: catalog(1, "fixture/cached") }).unwrap();
    let mut warming = catalog(2, "other/new");
    warming.unresolved_providers.push("fixture".into());
    c.message(ServerMessage::ModelCatalog { catalog: warming }).unwrap();
    assert_eq!(c.model_catalog.models.iter().map(|m| m.value.as_str()).collect::<Vec<_>>(), ["fixture/cached", "other/new"]);
    c.message(ServerMessage::ModelCatalog { catalog: catalog(3, "fixture/refreshed") }).unwrap();
    assert_eq!(c.model_catalog.models.iter().map(|m| m.value.as_str()).collect::<Vec<_>>(), ["fixture/refreshed"]);
}
