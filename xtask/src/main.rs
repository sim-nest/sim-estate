#![forbid(unsafe_code)]

use std::{fs, path::Path, process::Command};

fn files(root: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path
            .file_name()
            .is_some_and(|n| n == "target" || n == ".git")
        {
            continue;
        }
        if path.is_dir() {
            files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn contract() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("missing root")?;
    let mut paths = Vec::new();
    files(root, &mut paths)?;
    for path in &paths {
        if path.extension().is_some_and(|x| x == "rs") {
            let lines = fs::read_to_string(path)
                .map_err(|e| e.to_string())?
                .lines()
                .count();
            if lines > 500 {
                return Err(format!("{} has {lines} lines (limit 500)", path.display()));
            }
        }
    }
    let manifests = paths
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "Cargo.toml"));
    for manifest in manifests {
        let text = fs::read_to_string(manifest).map_err(|e| e.to_string())?;
        if text.contains("[package]")
            && !text.contains("name = \"xtask\"")
            && !text.contains("sim-pure = true")
            && !text.contains("sim-pure = false")
        {
            return Err(format!("{} is missing sim-pure = true", manifest.display()));
        }
    }
    let forbidden = [
        "std::process",
        "std::env",
        "std::fs",
        "std::net",
        "target_os",
        "cfg!(",
        "cfg_attr",
    ];
    for path in paths
        .iter()
        .filter(|p| p.components().any(|c| c.as_os_str() == "crates"))
    {
        let text = fs::read_to_string(path).unwrap_or_default();
        for needle in forbidden {
            if text.contains(needle) {
                return Err(format!("OS_5: {} contains {needle}", path.display()));
            }
        }
    }
    println!("OS_5: zero host facts; site crates depend only on the canonical portable port");
    Ok(())
}

fn run(name: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(name)
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} {args:?} failed"))
    }
}

fn main() {
    let result = match std::env::args().nth(1).as_deref() {
        Some("os5" | "check-file-sizes" | "simdoc") => contract(),
        Some("check") => contract()
            .and_then(|()| run("cargo", &["fmt", "--all", "--check"]))
            .and_then(|()| run("cargo", &["test", "--workspace"]))
            .and_then(|()| {
                run(
                    "cargo",
                    &[
                        "clippy",
                        "--workspace",
                        "--all-targets",
                        "--",
                        "-D",
                        "warnings",
                    ],
                )
            })
            .and_then(|()| run("cargo", &["doc", "--workspace", "--no-deps"])),
        _ => Err("usage: cargo run -p xtask -- check|os5|check-file-sizes|simdoc --check".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
