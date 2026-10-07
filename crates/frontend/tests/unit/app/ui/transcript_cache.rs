use super::*;
use super::super::ItemId;

fn layout(rows: usize) -> Layout {
    Layout { measured: (1, 1, 1), placed: (0..rows).map(|i| Placed {
        key: i.to_string(), item: ItemId::Thinking(i.to_string()), stamp: 0, exact: true,
        top: 0., height: 20., sender: tau_net::EventRole::Assistant, text_keys: vec![], overflow: 0.,
    }).collect() }
}

#[test]
fn retains_geometry_until_pressure_and_evicts_least_recent_chat() {
    let mut cache = Cache::default();
    cache.put("a".into(), layout(10));
    cache.put("b".into(), layout(10));
    cache.trim(10);
    assert_eq!(cache.entries.len(), 2);
    let a = cache.take("a").unwrap();
    cache.put("a".into(), a); // a was revisited, b is now the oldest.
    cache.trim(MAX_ROWS - 10);
    assert!(cache.take("b").is_none());
    assert!(cache.take("a").is_some());
    assert_eq!(cache.rows, 0);
}

#[test]
fn entry_budget_also_bounds_empty_chats_and_active_geometry_is_never_evicted() {
    let mut cache = Cache::default();
    for i in 0..MAX_CHATS + 2 { cache.put(i.to_string(), layout(0)); }
    cache.trim(0);
    assert_eq!(cache.entries.len(), MAX_CHATS);
    assert!(cache.take("0").is_none());
    cache.put("oversize".into(), layout(MAX_ROWS + 1));
    cache.trim(0);
    assert!(cache.entries.is_empty());
    assert_eq!(cache.rows, 0);
}
