use super::slash_suggestions;
use tau_net::{ModelCatalog, ModelSuggestion};

#[test]
fn builtin_completion_is_available_without_loading_a_chat_or_contacting_a_daemon() {
    let catalog=ModelCatalog::default();
    assert_eq!(slash_suggestions("",&catalog),["/compact ","/model ","/thinking ","/name ","/fast "]);
    assert_eq!(slash_suggestions("th",&catalog),["/thinking "]);
    assert!(slash_suggestions("extension",&catalog).is_empty());
    assert!(slash_suggestions("name title",&catalog).is_empty());
}

#[test]
fn argument_completion_uses_shared_builtin_values_and_the_account_catalogue() {
    let catalog=ModelCatalog { models:vec![ModelSuggestion {value:"fixture/model".into(),description:None}],..Default::default() };
    assert_eq!(slash_suggestions("model fix",&catalog),["/model fixture/model"]);
    assert_eq!(slash_suggestions("thinking x",&catalog),["/thinking xhigh"]);
    assert_eq!(slash_suggestions("fast ",&catalog),["/fast on","/fast off","/fast status"]);
    assert!(slash_suggestions("model missing",&catalog).is_empty());
}

#[test]
fn a_large_account_catalogue_only_builds_visible_suggestions() {
    let catalog=ModelCatalog { models:(0..20_000).map(|i|ModelSuggestion {value:format!("fixture/{i}"),description:None}).collect(),..Default::default() };
    assert_eq!(slash_suggestions("model ",&catalog),["/model fixture/0","/model fixture/1","/model fixture/2","/model fixture/3","/model fixture/4"]);
}
