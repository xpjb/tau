use super::*;
#[test]
fn ime_viewport_is_not_subtracted_twice_and_restores_after_hide() {
    let visible = [0, 24, 360, 420];
    assert_eq!(viewport((360, 800), [0, 24, 360, 800], visible), [0, 24, 360, 420]);
    assert_eq!(viewport((360, 420), [0, 24, 360, 420], visible), [0, 24, 360, 420]);
    assert_eq!(viewport((360, 800), [0, 24, 360, 776], [0, 24, 360, 776]), [0, 24, 360, 776]);
    assert_eq!(viewport((800, 360), [0; 4], [30, 0, 776, 180]), [30, 0, 776, 180]);
}
