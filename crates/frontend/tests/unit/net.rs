use super::*;
#[test]
fn minimum_start_spacing_is_not_a_post_failure_wait_or_growing_backoff() {
    let now = Instant::now();
    for elapsed in [10, 999, 1000, 2000, 5000, 43000] {
        let failed = now + Duration::from_millis(elapsed);
        assert_eq!(next_attempt(now, failed), now + Duration::from_millis(elapsed.max(1000)));
    }
}
