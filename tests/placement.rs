use rustleblower::prompts::placement::{Position, Rect};
#[test]
fn corners_use_work_area_margins_and_negative_monitor_origins() {
    let area = Rect {
        x: -1920.,
        y: 40.,
        width: 1920.,
        height: 1040.,
    };
    for (key, x, y) in [
        ("center", -1285., 315.),
        ("top-left", -1904., 56.),
        ("top-right", -666., 56.),
        ("bottom-left", -1904., 574.),
        ("bottom-right", -666., 574.),
    ] {
        let bounds = Position::parse(key)
            .unwrap()
            .place(area, 650., 490.)
            .unwrap();
        assert_eq!(
            bounds,
            Rect {
                x,
                y,
                width: 650.,
                height: 490.
            }
        );
    }
}
#[test]
fn expanded_dialogs_fit_small_workspaces_and_invalid_geometry_is_rejected() {
    let area = Rect {
        x: 100.,
        y: 200.,
        width: 600.,
        height: 400.,
    };
    for (key, _) in rustleblower::prompts::placement::POSITIONS {
        let bounds = Position::parse(key)
            .unwrap()
            .place(area, 860., 780.)
            .unwrap();
        assert!(bounds.x >= area.x && bounds.y >= area.y);
        assert!(
            bounds.x + bounds.width <= area.x + area.width
                && bounds.y + bounds.height <= area.y + area.height
        );
        assert_eq!((bounds.width, bounds.height), (568., 368.));
    }
    assert!(
        Position::Center
            .place(Rect { width: 0., ..area }, 650., 490.)
            .is_none()
    );
    assert!(Position::Center.place(area, f32::NAN, 490.).is_none());
    assert!(Position::parse("anywhere").is_none());
}
