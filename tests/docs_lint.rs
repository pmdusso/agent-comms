use comms::roster::{prompt_problems, render};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> &'static Path { Path::new(env!("CARGO_MANIFEST_DIR")) }

fn markdown_files(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.to_string_lossy().ends_with(suffix))
        .collect();
    files.sort();
    files
}

fn skill_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root().join("skills")).unwrap().filter_map(std::result::Result::ok) {
        let path = entry.path().join("SKILL.md");
        if path.is_file() { files.push(path); }
    }
    files.sort();
    files
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap().filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn prompts_and_skills_avoid_non_portable_literals() {
    let prohibited = [
        (r"/Users/", "caminho absoluto de uma máquina"),
        (r"\b(main|master)\b", "nome de branch fixo — use $(comms trunk)"),
        (r"-R\s+[\w.-]+/[\w.-]+", "repo fixo no gh — deixe o gh resolver"),
        (r"PVT[A-Z]*_", "node id de GitHub Project"),
        (r"--(project|field)-id\s+\S", "id de board fixo — use board-item/board-set"),
        (r"\b(pytest|uvx|ruff|make test|npm (test|run)|yarn test|go test|cargo test)\b|\./test-all\b", "runner de teste fixo — use $(comms testcmd)"),
    ];
    let patterns: Vec<(Regex, &str)> = prohibited.into_iter().map(|(pattern, reason)| (Regex::new(pattern).unwrap(), reason)).collect();
    let mut files = markdown_files(&root().join("prompts"), ".md");
    files.extend(skill_files());
    let mut leaks = Vec::new();
    for file in files {
        for (line_number, line) in fs::read_to_string(&file).unwrap().lines().enumerate() {
            for (pattern, reason) in &patterns {
                if pattern.is_match(line) && !line.contains("comms testcmd") {
                    leaks.push(format!("{}:{}: {reason} → {}", file.display(), line_number + 1, line.trim()));
                }
            }
        }
    }
    assert!(leaks.is_empty(), "literais não-portáteis:\n{}", leaks.join("\n"));
}

#[test]
fn prompts_and_adhoc_template_keep_required_structure() {
    for file in markdown_files(&root().join("prompts"), ".md") {
        let problems = prompt_problems(&fs::read_to_string(&file).unwrap());
        assert!(problems.is_empty(), "{}: {}", file.display(), problems.join("; "));
    }
    let template = fs::read_to_string(root().join("templates/agent_prompt.md")).unwrap();
    let rendered = render(&template, &[("NOME", "DBA"), ("nome", "dba"), ("por", "arch"), ("descricao", "x")]);
    assert!(prompt_problems(&rendered).is_empty());
    assert!(!rendered.contains("{{"));
    let pm = fs::read_to_string(root().join("prompts/pm.md")).unwrap().to_lowercase();
    for field in ["objetivo", "critérios", "fronteiras"] {
        assert!(pm.contains(field), "pm.md não cita {field}");
    }
}

#[test]
fn rust_sources_do_not_embed_project_specific_ids() {
    let needles = [concat!("gear", "pal"), concat!("PVT", "_kwHO"), concat!("PVTSS", "F_")];
    for file in rust_files(&root().join("src")) {
        let text = fs::read_to_string(&file).unwrap();
        for needle in needles {
            assert!(!text.contains(needle), "{} contém ID específico", file.display());
        }
    }
}

#[test]
fn live_docs_never_invoke_python_engine() {
    let mut files = markdown_files(&root().join("prompts"), ".md");
    files.extend(skill_files());
    files.extend(markdown_files(&root().join("templates"), ".md"));
    files.push(root().join("README.md"));
    files.push(root().join("docs/SETUP_GUIDE.md"));
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        assert!(!text.contains("./comms.py") && !text.contains("python3 comms.py"), "{} ainda chama o engine Python", file.display());
    }
}
