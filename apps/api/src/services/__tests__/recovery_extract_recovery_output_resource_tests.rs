use super::{recovery_validation_fixtures::*, *};

#[test]
fn deep_rollback_path_ownership_does_not_overflow_the_stack() {
    let mut path = None;
    for _ in 0..50_000 {
        path = Some(Rc::new(OutputPath {
            parent: path,
            name: "part".into(),
        }));
    }
    drop(path);
}

#[test]
fn directory_creation_observes_the_deadline_before_each_component() {
    let root = tempfile::tempdir().unwrap();
    let mut config = limits();
    config.restore_deadline_seconds = 0;
    let mut output = ExtractionOutput::new(root.path(), &config, Instant::now()).unwrap();
    assert!(matches!(
        output.directory("vehicle-image-cache/nested"),
        Err(AppError::RecoveryDeadline(_))
    ));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
