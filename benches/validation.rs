use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use deliver::{expand_paths, quick_check_files, FileCheck, GitCheck, Spec};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn deliver_binary() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    Path::new(&manifest).join("target/release/deliver")
}

fn make_file_check(path: &str) -> FileCheck {
    FileCheck {
        path: path.to_string(),
        required: true,
        max_size_bytes: None,
        min_size_bytes: None,
        forbid_regex: Vec::new(),
        require_regex: Vec::new(),
        require_line_count: None,
        encoding: None,
        forbid_symlinks: false,
        sha256: None,
        blake3: None,
        require_license_header: None,
        json_schema: None,
        require_toml_keys: Vec::new(),
        require_json_keys: Vec::new(),
        yaml_schema: None,
        check_references: false,
    }
}

fn make_spec(files: Vec<FileCheck>) -> Spec {
    Spec {
        extends: None,
        files,
        commands: Vec::new(),
        git: GitCheck::default(),
        directories: Vec::new(),
        metadata: std::collections::HashMap::new(),
    }
}

fn bench_glob_expansion(c: &mut Criterion) {
    let mut group = c.benchmark_group("glob_expansion");

    for file_count in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            file_count,
            |b, &count| {
                let dir = TempDir::new().unwrap();

                for i in 0..count {
                    let path = dir.path().join(format!("{}.rs", i));
                    fs::write(&path, "fn main() {}").unwrap();
                }

                // Add some non-matching files
                for i in 0..(count / 2) {
                    let path = dir.path().join(format!("{}.txt", i));
                    fs::write(&path, "text").unwrap();
                }

                b.iter(|| {
                    expand_paths(black_box("*.rs"), black_box(dir.path()));
                });
            },
        );
    }
    group.finish();
}

fn bench_quick_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("quick_check");

    for file_count in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            file_count,
            |b, &count| {
                let dir = TempDir::new().unwrap();
                let mut paths = Vec::new();

                for i in 0..count {
                    let path = dir.path().join(format!("{}.txt", i));
                    fs::write(&path, "content").unwrap();
                    paths.push(path);
                }

                b.iter(|| {
                    quick_check_files(black_box(&paths), black_box(dir.path()));
                });
            },
        );
    }
    group.finish();
}

fn bench_spec_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("spec_validation");

    for check_count in [5, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(check_count),
            check_count,
            |b, &count| {
                let dir = TempDir::new().unwrap();
                let mut files = Vec::new();

                for i in 0..count {
                    let path = dir.path().join(format!("{}.txt", i));
                    fs::write(&path, "content").unwrap();

                    files.push(make_file_check(&format!("{}.txt", i)));
                }

                let spec = make_spec(files);

                b.iter(|| {
                    spec.validate(black_box(dir.path()));
                });
            },
        );
    }
    group.finish();
}

fn bench_cli_end_to_end(c: &mut Criterion) {
    let mut group = c.benchmark_group("cli_end_to_end");
    let binary = deliver_binary();

    for count in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            let dir = TempDir::new().unwrap();
            let mut files = Vec::new();

            for i in 0..count {
                let name = format!("{}.txt", i);
                fs::write(dir.path().join(&name), "deliverable content").unwrap();
                files.push(make_file_check(&name));
            }

            let spec = make_spec(files);
            let spec_path = dir.path().join("spec.toml");
            fs::write(&spec_path, toml::to_string(&spec).unwrap()).unwrap();

            b.iter(|| {
                let output = Command::new(&binary)
                    .arg("--spec")
                    .arg(&spec_path)
                    .arg("--base")
                    .arg(dir.path())
                    .arg("--color")
                    .arg("never")
                    .arg("--progress")
                    .arg("never")
                    .output()
                    .unwrap();
                assert!(output.status.success(), "deliver CLI failed");
            });
        });
    }
    group.finish();
}

fn bench_check_complexity(c: &mut Criterion) {
    let mut group = c.benchmark_group("check_complexity");
    let binary = deliver_binary();
    let count = 100;

    type SetupFn = Box<dyn Fn(&TempDir) -> Vec<FileCheck>>;
    let setups: Vec<(&str, SetupFn)> = vec![
        (
            "existence",
            Box::new(move |dir| {
                (0..count)
                    .map(|i| {
                        let name = format!("{}.txt", i);
                        fs::write(dir.path().join(&name), "deliverable content").unwrap();
                        make_file_check(&name)
                    })
                    .collect()
            }),
        ),
        (
            "regex",
            Box::new(move |dir| {
                (0..count)
                    .map(|i| {
                        let name = format!("{}.txt", i);
                        fs::write(dir.path().join(&name), "deliverable content\nOK").unwrap();
                        FileCheck {
                            require_regex: vec!["OK".to_string()],
                            forbid_regex: vec!["TODO".to_string()],
                            ..make_file_check(&name)
                        }
                    })
                    .collect()
            }),
        ),
        (
            "sha256",
            Box::new(move |dir| {
                (0..count)
                    .map(|i| {
                        let name = format!("{}.txt", i);
                        let content = b"deliverable content";
                        fs::write(dir.path().join(&name), content).unwrap();
                        let mut hasher = Sha256::new();
                        hasher.update(content);
                        let hash = format!("{:x}", hasher.finalize());
                        FileCheck {
                            sha256: Some(hash),
                            ..make_file_check(&name)
                        }
                    })
                    .collect()
            }),
        ),
        (
            "json_schema",
            Box::new(move |dir| {
                let schema = r#"{"type":"object","properties":{"name":{"type":"string"},"value":{"type":"number"}},"required":["name","value"]}"#;
                (0..count)
                    .map(|i| {
                        let name = format!("{}.json", i);
                        fs::write(dir.path().join(&name), r#"{"name":"x","value":1}"#).unwrap();
                        FileCheck {
                            json_schema: Some(schema.to_string()),
                            ..make_file_check(&name)
                        }
                    })
                    .collect()
            }),
        ),
        (
            "toml_keys",
            Box::new(move |dir| {
                (0..count)
                    .map(|i| {
                        let name = format!("{}.toml", i);
                        fs::write(
                            dir.path().join(&name),
                            "[package]\nname = \"x\"\nversion = \"1.0.0\"\n",
                        )
                        .unwrap();
                        FileCheck {
                            require_toml_keys: vec![
                                "package.name".to_string(),
                                "package.version".to_string(),
                            ],
                            ..make_file_check(&name)
                        }
                    })
                    .collect()
            }),
        ),
    ];

    for (label, setup) in setups {
        group.bench_with_input(BenchmarkId::new("deliver", label), label, |b, _| {
            let dir = TempDir::new().unwrap();
            let files = setup(&dir);
            let spec = make_spec(files);
            let spec_path = dir.path().join("spec.toml");
            fs::write(&spec_path, toml::to_string(&spec).unwrap()).unwrap();

            b.iter(|| {
                let output = Command::new(&binary)
                    .arg("--spec")
                    .arg(&spec_path)
                    .arg("--base")
                    .arg(dir.path())
                    .arg("--color")
                    .arg("never")
                    .arg("--progress")
                    .arg("never")
                    .output()
                    .unwrap();
                assert!(output.status.success(), "deliver CLI failed");
            });
        });
    }
    group.finish();
}

fn bench_deliver_vs_shell(c: &mut Criterion) {
    let mut group = c.benchmark_group("deliver_vs_shell");
    let binary = deliver_binary();

    for count in [10, 100].iter() {
        group.bench_with_input(BenchmarkId::new("deliver", count), count, |b, &count| {
            let dir = TempDir::new().unwrap();
            let mut files = Vec::new();

            for i in 0..count {
                let name = format!("{}.txt", i);
                fs::write(dir.path().join(&name), "deliverable content\nOK").unwrap();
                files.push(FileCheck {
                    require_regex: vec!["OK".to_string()],
                    forbid_regex: vec!["TODO".to_string()],
                    ..make_file_check(&name)
                });
            }

            let spec = make_spec(files);
            let spec_path = dir.path().join("spec.toml");
            fs::write(&spec_path, toml::to_string(&spec).unwrap()).unwrap();

            b.iter(|| {
                Command::new(&binary)
                    .arg("--spec")
                    .arg(&spec_path)
                    .arg("--base")
                    .arg(dir.path())
                    .arg("--color")
                    .arg("never")
                    .arg("--progress")
                    .arg("never")
                    .output()
                    .unwrap();
            });
        });

        group.bench_with_input(BenchmarkId::new("shell", count), count, |b, &count| {
            let dir = TempDir::new().unwrap();
            let mut script = String::from("#!/bin/sh\nset -e\n");

            for i in 0..count {
                let name = format!("{}.txt", i);
                fs::write(dir.path().join(&name), "deliverable content\nOK").unwrap();
                script.push_str(&format!(
                    "test -f {0} || exit 1\ngrep -q 'OK' {0} || exit 1\ngrep -q 'TODO' {0} && exit 1\n",
                    name
                ));
            }

            let script_path = dir.path().join("check.sh");
            fs::write(&script_path, script).unwrap();

            b.iter(|| {
                Command::new("sh")
                    .arg(&script_path)
                    .current_dir(dir.path())
                    .output()
                    .unwrap();
            });
        });
    }
    group.finish();
}

fn bench_spec_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("spec_parse");

    for count in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            let files: Vec<FileCheck> = (0..count)
                .map(|i| make_file_check(&format!("{}.txt", i)))
                .collect();
            let spec = make_spec(files);
            let toml_text = toml::to_string(&spec).unwrap();

            b.iter(|| {
                let _spec = Spec::from_toml(black_box(&toml_text)).unwrap();
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_glob_expansion,
    bench_quick_check,
    bench_spec_validation,
    bench_cli_end_to_end,
    bench_check_complexity,
    bench_deliver_vs_shell,
    bench_spec_parse
);
criterion_main!(benches);
