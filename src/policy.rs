pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const KINDS: [&str; 6] = ["mensagem", "decisao", "bloqueio", "handoff", "pergunta", "gate"];
pub const STATUSES: [&str; 4] = ["aberto", "bloqueado", "feito", "debito"];
pub const SEVERITIES: [&str; 3] = ["grave", "medio", "pequeno"];
pub const DEFAULT_ROSTER: [&str; 5] = ["arch", "pm", "dev", "tester", "reviewer"];
pub const HUMAN: &str = "human";
pub const BROADCAST: &str = "todos";
pub const CROSS: &str = "cross";
pub const RESERVED_NAMES: [&str; 3] = [HUMAN, BROADCAST, CROSS];
pub const HANDOFF_FIELDS: [&str; 3] = ["objetivo", "criterios", "fronteiras"];
pub const PROMPT_SECTIONS: [&str; 4] = ["## Antes de agir", "## Fluxo", "## Limites", "## Término"];
pub const MAX_NUNCA: usize = 5;
pub const TEST_CMD_HINTS: [(&str, &str); 5] = [
    ("Makefile", "make test"),
    ("pyproject.toml", "uv run pytest -q"),
    ("package.json", "npm test"),
    ("Cargo.toml", "cargo test"),
    ("go.mod", "go test ./..."),
];

pub struct Roles {
    pub executor: &'static str,
    pub panel: [&'static str; 5],
    pub pr_bot: &'static str,
    pub advisor: &'static str,
}

pub const ROLES: Roles = Roles {
    executor: "claude",
    panel: ["gemini", "codex", "claude", "minimax", "qwen"],
    pr_bot: "auto",
    advisor: "claude",
};
