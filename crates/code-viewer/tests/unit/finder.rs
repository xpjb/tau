use super::*;
#[test]
fn forgiving_paths_unordered_words_boundaries_and_extended_terms() {
    for (query, path) in [("fntapcdvw", "frontend/src/app/code_view.rs"), ("main src", "src/main.rs"),
        ("cafe", "src/café.rs"), ("src !test .rs$", "src/main.rs"), ("'code_view", "frontend/src/app/code_view.rs")] {
        assert!(Finder::new(query).score(path).is_some(), "{query}: {path}");
    }
    assert!(Finder::new("src !test").score("src/test.rs").is_none());
    assert!(Finder::new("Main").score("src/main.rs").is_none());
    assert!(Finder::new("main").score("src/main.rs") > Finder::new("main").score("many/arbitrary/intermediate/names.rs"));
    for path in ["src/cafe\u{301}_👩‍💻.rs", "src/é👩‍💻.rs"] {
        let ranges = Finder::new(".rs").highlights(path);
        assert_eq!(ranges.iter().map(|r| &path[r.clone()]).collect::<String>(), ".rs");
    }
    let path = "src/café_🦀.rs";
    let ranges = Finder::new("cafe 🦀").highlights(path);
    assert_eq!(ranges.iter().map(|r| &path[r.clone()]).collect::<Vec<_>>(), ["café", "🦀"]);
}
