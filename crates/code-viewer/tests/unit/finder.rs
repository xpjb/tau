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

#[test]
fn wire_index_names_join_daemon_native_roots_independently_of_client_os() {
    use tau_net::files::IndexedPath;
    for (root, expected) in [("/work", "/work/src/a.rs"), (r"\\?\C:\work", r"\\?\C:\work\src\a.rs"),
        (r"\\?\UNC\server\share", r"\\?\UNC\server\share\src\a.rs")] {
        let index = PathIndex {root:root.into(),revision:String::new(),entries:vec![IndexedPath {path:"src/a.rs".into(),symlink:false}],visible:1};
        assert_eq!(index.absolute(0),expected);
    }
}
