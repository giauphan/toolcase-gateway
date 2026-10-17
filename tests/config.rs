use toolcase_gateway::config::{parse_muse_approval_mode, MuseApprovalMode};

#[test]
fn approval_mode_on_enables_scoped_auto_policy() {
    assert_eq!(parse_muse_approval_mode("on"), MuseApprovalMode::ScopedAuto);
}

#[test]
fn approval_mode_scoped_auto_enables_same_policy() {
    assert_eq!(
        parse_muse_approval_mode("scoped_auto"),
        MuseApprovalMode::ScopedAuto
    );
}

#[test]
fn approval_mode_observe_enables_observe_policy() {
    assert_eq!(
        parse_muse_approval_mode("observe"),
        MuseApprovalMode::Observe
    );
}

#[test]
fn approval_mode_unknown_remains_off() {
    assert_eq!(
        parse_muse_approval_mode("anything-else"),
        MuseApprovalMode::Off
    );
}

#[test]
fn approval_mode_whitespace_and_case_are_tolerated() {
    assert_eq!(
        parse_muse_approval_mode("  ON  "),
        MuseApprovalMode::ScopedAuto
    );
}
