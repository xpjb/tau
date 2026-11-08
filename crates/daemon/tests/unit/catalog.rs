use super::*;

#[tokio::test]
async fn old_catalog_revalidates_without_discarding_the_last_good_limits() {
    let root = tempfile::tempdir().unwrap();
    let settings = Settings::default();
    let config = &settings.providers["openai-codex"];
    let catalog = ModelCatalog::load(root.path().join("model-catalog.json")).await;
    let model = settings.agent.model.clone();
    assert!(catalog.begin(&model.provider, config, None, false));
    catalog.save(&model.provider, config, "fixture-identity".into(), HashMap::from([(model.model_id.clone(), 272_000)])).await.unwrap();
    let saved = tokio::fs::read(&catalog.path).await.unwrap();
    {
        let mut state = catalog.state.lock().unwrap();
        let record = state.records.get_mut(&model.provider).unwrap();
        record.saved.as_mut().unwrap().fetched_at_ms = crate::agent::now_ms() - 3_600_000;
        record.retry_after = None;
    }
    assert_eq!(catalog.capacity(&settings, &model), Some(272_000));
    assert!(catalog.begin(&model.provider, config, None, false), "An old valid catalog must be revalidated automatically");
    assert!(!catalog.begin(&model.provider, config, None, true), "Concurrent requests coalesce, including manual refresh");
    assert!(!catalog.restore(&model.provider, config, "fixture-identity", None), "A stale restore must not skip the GET");
    catalog.failed(&model.provider);
    assert_eq!(catalog.capacity(&settings, &model), Some(272_000), "Age and transient failures do not expire valid limits");
    assert_eq!(tokio::fs::read(&catalog.path).await.unwrap(), saved);
    assert!(!catalog.begin(&model.provider, config, None, false), "Failures are rate limited");
    assert!(catalog.begin(&model.provider, config, None, true), "Manual refresh can bypass the cooldown");
}

#[tokio::test]
async fn missing_exact_ids_revalidate_but_repeated_misses_do_not_hammer_the_provider() {
    let root = tempfile::tempdir().unwrap();
    let settings = Settings::default();
    let config = &settings.providers["openai-codex"];
    let catalog = ModelCatalog::load(root.path().join("model-catalog.json")).await;
    let model = settings.agent.model.clone();
    catalog.begin(&model.provider, config, None, false);
    let windows = HashMap::from([(model.model_id.clone(), 272_000)]);
    catalog.save(&model.provider, config, "fixture-identity".into(), windows.clone()).await.unwrap();
    // A catalog fetched moments ago need not be fetched again for a miss.
    assert!(!catalog.begin(&model.provider, config, Some("gpt-6.1-sol"), false));
    catalog.state.lock().unwrap().records.get_mut(&model.provider).unwrap().retry_after = None;
    assert!(!catalog.begin(&model.provider, config, Some(&model.model_id), false));
    assert!(catalog.begin(&model.provider, config, Some("gpt-6.1-sol"), false));
    assert!(!catalog.restore(&model.provider, config, "fixture-identity", Some("gpt-6.1-sol")), "A fresh cache missing the requested ID must not skip the GET");
    assert_eq!(catalog.capacity(&settings, &model), Some(272_000));
    catalog.save(&model.provider, config, "fixture-identity".into(), windows).await.unwrap();
    assert!(!catalog.begin(&model.provider, config, Some("gpt-6.1-sol"), false));
    assert!(!catalog.begin(&model.provider, config, Some("another-missing-id"), false), "Miss cooldown is shared by all clients/models of the provider");
    let unknown = SessionModel { provider:model.provider.clone(), model_id:"gpt-6.1-sol".into() };
    assert_eq!(catalog.capacity(&settings, &unknown), None, "Never guess a model's context window");
    assert!(catalog.begin(&model.provider, config, None, true));
}

#[tokio::test]
async fn legacy_client_version_revalidates_and_restore_remains_identity_scoped() {
    let root = tempfile::tempdir().unwrap();
    let settings = Settings::default();
    let config = &settings.providers["openai-codex"];
    let model = settings.agent.model.clone();
    let path = root.path().join("model-catalog.json");
    let catalog = ModelCatalog::load(path.clone()).await;
    catalog.begin(&model.provider, config, None, false);
    catalog.save(&model.provider, config, "fixture-identity".into(), HashMap::from([(model.model_id.clone(), 272_000)])).await.unwrap();
    let saved = tokio::fs::read(&path).await.unwrap();
    for version in [None, Some("0.156.1"), Some(CODEX_CATALOG_CLIENT_VERSION)] {
        let mut file:Value = serde_json::from_slice(&saved).unwrap();
        let record = file["providers"][0].as_object_mut().unwrap();
        record.remove("codexClientVersion");
        if let Some(version) = version { record.insert("codexClientVersion".into(), Value::from(version)); }
        tokio::fs::write(&path, serde_json::to_vec(&file).unwrap()).await.unwrap();
        let restored = ModelCatalog::load(path.clone()).await;
        assert_eq!(restored.capacity(&settings, &model), None, "Disk metadata must be authorized before use");
        assert!(restored.begin(&model.provider, config, Some(&model.model_id), false));
        assert_eq!(restored.restore(&model.provider, config, "fixture-identity", Some(&model.model_id)), version == Some(CODEX_CATALOG_CLIENT_VERSION));
        assert_eq!(restored.capacity(&settings, &model), Some(272_000), "Old client-version metadata remains usable during revalidation");
        assert!(!restored.restore(&model.provider, config, "different-login", None));
        assert_eq!(restored.capacity(&settings, &model), None);
        let mut changed = config.clone(); changed.base_url.push_str("/different-endpoint");
        assert!(!restored.restore(&model.provider, &changed, "fixture-identity", None));
        restored.state.lock().unwrap().records.get_mut(&model.provider).unwrap().saved.as_mut().unwrap().fetched_at_ms = u64::MAX;
        assert!(!restored.restore(&model.provider, config, "fixture-identity", None), "A future timestamp is not permanently fresh");
    }
}
