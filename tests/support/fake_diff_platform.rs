use std::{env, fs, path::Path, process};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let value = |key: &str| {
        args.windows(2)
            .find(|pair| pair[0] == key)
            .map(|pair| pair[1].as_str())
    };
    fs::write(env::var_os("DIFF_ARGV").unwrap(), args.join("\n")).unwrap();
    let output = env::var("DIFF_OUTPUT").unwrap();
    if output == "echo_password" {
        println!("platform refused {}", value("/P").unwrap());
        process::exit(7);
    }
    if args.first().is_some_and(|arg| arg == "infobase") {
        assert!(args
            .windows(3)
            .any(|args| args == ["config", "export", "status"]));
        let base = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--base="))
            .unwrap();
        record_baseline(Path::new(base));
        let list = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--out="))
            .unwrap();
        match output.as_str() {
            "missing" => {}
            "empty" => fs::write(list, []).unwrap(),
            _ => fs::write(
                list,
                format!("\u{feff}{}", output.replace("\\n", "\r\n")),
            )
            .unwrap(),
        }
        return;
    }
    let target = if let Some(list) = value("-getChanges") {
        record_baseline(
            &Path::new(value("/DumpConfigToFiles").unwrap()).join("ConfigDumpInfo.xml"),
        );
        list
    } else {
        if let Some(file) = value("-SecondFile") {
            assert_eq!(fs::read(file).unwrap(), b"package");
        }
        value("-ReportFile").unwrap()
    };
    assert!(!Path::new(target).exists(), "output path must start absent");
    match output.as_str() {
        "missing" => {}
        "empty" => fs::write(target, []).unwrap(),
        "blank" => fs::write(target, b"\r\n").unwrap(),
        "partial" => {
            fs::write(target, b"partial").unwrap();
            process::exit(7);
        }
        "report" => fs::write(target, b"\xff\xfeR\0\r\0\n\0").unwrap(),
        _ => fs::write(target, format!("\u{feff}{}", output.replace("\\n", "\r\n"))).unwrap(),
    }
}

fn record_baseline(path: &Path) {
    fs::copy(path, env::var_os("DIFF_BASELINE").unwrap()).unwrap();
}
