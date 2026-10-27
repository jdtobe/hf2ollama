use roost_core::persistence::atomic_write;
use std::fs;
use std::sync::{Arc, Barrier};

#[test]
fn creates_nested_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one").join("two").join("metadata");

    atomic_write(&path, b"metadata").unwrap();

    assert_eq!(fs::read(&path).unwrap(), b"metadata");
}

#[test]
fn round_trips_complete_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata");
    let bytes = b"first line\n\0second line\n";

    atomic_write(&path, bytes).unwrap();

    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn rename_failure_preserves_existing_destination() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("keep"), b"original").unwrap();

    assert!(atomic_write(&path, b"replacement").is_err());

    assert!(path.is_dir());
    assert_eq!(fs::read(path.join("keep")).unwrap(), b"original");
}

#[test]
fn failed_rename_cleans_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata");
    fs::create_dir(&path).unwrap();

    assert!(atomic_write(&path, b"replacement").is_err());

    assert!(fs::read_dir(dir.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("metadata.roost-")));
}

#[test]
fn concurrent_writers_leave_one_complete_json_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = Arc::new(dir.path().join("metadata.json"));
    let start = Arc::new(Barrier::new(16));
    let mut writers = Vec::new();

    for writer in 0..16 {
        let path = Arc::clone(&path);
        let start = Arc::clone(&start);
        writers.push(std::thread::spawn(move || {
            start.wait();
            let bytes = format!(r#"{{"writer":{writer},"payload":"complete"}}"#);
            atomic_write(&path, bytes.as_bytes()).unwrap();
        }));
    }

    for writer in writers {
        writer.join().unwrap();
    }

    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&*path).unwrap()).unwrap();
    assert!(saved["writer"].as_u64().is_some_and(|writer| writer < 16));
    assert_eq!(saved["payload"], "complete");
}
