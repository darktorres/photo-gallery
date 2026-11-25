//! A pinned run takes its reader count from the command line, and the pipeline's record of the run carries that count. The test runs the
//! built `pg-driver` binary, as a script would.

use std::process::Command;

#[test]
fn a_pinned_reader_count_is_the_one_the_pipeline_reads_with() {
    let dir = pg_testkit::run_folder_with("pg_driver_tests", "pin-readers", &["home", "images"]).unwrap();
    for name in ["a.png", "b.png", "c.png"] {
        std::fs::write(dir.join("images").join(name), pg_codec::fixtures::png_rgb(8, 8, &[0; 192]).unwrap()).unwrap();
    }
    let perf = dir.join("perf.jsonl");
    let output = Command::new(env!("CARGO_BIN_EXE_pg-driver"))
        .arg("run")
        .arg("--home")
        .arg(dir.join("home"))
        .arg("--create")
        .arg("--add")
        .arg(dir.join("images"))
        .args(["--pin-readers", "2", "--timeout", "120"])
        .arg("--perf")
        .arg(&perf)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let text = std::fs::read_to_string(&perf).unwrap();
    let readers: Vec<u64> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|record| record["name"] == "pipeline")
        .filter_map(|record| record["fields"]["readers"].as_u64())
        .collect();
    assert!(!readers.is_empty(), "the run's passes record their pipelines:\n{text}");
    assert!(readers.iter().all(|&count| count == 2), "every pipeline reads with the pinned two readers: {readers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_home_is_refused_unless_it_is_created() {
    let dir = pg_testkit::run_folder("pg_driver_tests", "missing-home").unwrap();
    let home = dir.join("nowhere");
    let refused = Command::new(env!("CARGO_BIN_EXE_pg-driver")).arg("status").arg("--home").arg(&home).output().unwrap();
    assert!(!refused.status.success(), "a missing home is refused");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--create"), "the refusal says how to make it");
    assert!(!home.exists(), "nothing is made without --create");

    let created = Command::new(env!("CARGO_BIN_EXE_pg-driver"))
        .args(["status", "--create", "--home"])
        .arg(&home)
        .output()
        .unwrap();
    assert!(created.status.success(), "{}", String::from_utf8_lossy(&created.stderr));
    assert!(home.join("catalog").is_dir(), "with --create the home is made");
    let _ = std::fs::remove_dir_all(&dir);
}
