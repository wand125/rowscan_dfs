use std::process::Command;
#[test]
fn cube_cli_validation_and_determinism() {
    let bin = env!("CARGO_BIN_EXE_rowscan_dfs");
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/reference/ex3.txt");
    let out = std::env::temp_dir().join(format!("rowscan_dfs-cubes-{}.json", std::process::id()));
    let path = out.to_str().unwrap();
    for args in [
        vec!["--cubes", "0", "--cube-json", path],
        vec!["--cubes", "3"],
        vec!["--cube-json", path],
        vec!["--cubes", "3", "--cube-json", path, "--direction", "1"],
        vec!["--cubes", "3", "--cube-json", path, "--race", "1"],
    ] {
        assert_eq!(
            Command::new(bin)
                .args(args)
                .arg(file)
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
    }
    let mut first = None;
    for memo in ["0", "26"] {
        let r = Command::new(bin)
            .args(["--cubes", "64", "--cube-json", path, "--memo", memo, file])
            .output()
            .unwrap();
        assert!(r.status.success(), "{:?}", r);
        assert!(r.stderr.is_empty());
        assert!(String::from_utf8(r.stdout)
            .unwrap()
            .contains("cube solutions:"));
        let data = std::fs::read_to_string(&out).unwrap();
        let prefix = data.split(",\"elapsed_ms\":").next().unwrap().to_owned();
        if let Some(ref expected) = first {
            assert_eq!(expected, &prefix);
        } else {
            first = Some(prefix);
        }
    }
    std::fs::remove_file(out).unwrap();
}
