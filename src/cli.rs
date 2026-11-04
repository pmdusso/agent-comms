use clap::{Args, Parser, Subcommand};
use clap::builder::PossibleValuesParser;
use crate::policy::{KINDS, STATUSES};

pub const USO: &str = "comms — canal estruturado dos agentes do projeto (SQLite em comms.db).\n\nUso rápido:\n  comms state                                  # estado atual (kv + itens + turnos abertos)\n  comms post --de arch --para todos --tipo mensagem \\\n      --titulo \"...\" --corpo \"...\" [--refs \"TRAG-POC.md MKT-153\"]\n  comms turns [--all]        comms show N        comms resolve N\n  comms items [--all]        comms item-add --titulo \"...\" --dono tester\n  comms item-set N --status feito [--bloqueio \"...\"]\n  comms set bola_com \"tester\"\n  comms agents                                 # roster (= docs/prompts/*.md + agentes ad-hoc)\n  comms brief dev                              # prompt + estado, num blob só\n  comms gate [--refs issue:42]                 # F4/G3/G4; exit 1 se reprovar\n  comms trunk    comms testcmd                 # tronco e suíte, sem chutar\n  comms bootstrap                              # inicializar repositório (db + manual)\n  comms agent-add dba --descricao \"...\" --por arch\n  comms --json <cmd>                           # saída estruturada para agentes\n\nRegra de ouro LGPD: nada de telefone/CPF/coordenada de cliente aqui.";

#[derive(Parser)]
#[command(name = "comms", version, infer_long_args = true, about = "canal estruturado dos agentes", long_about = USO)]
pub struct Cli {
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    #[command(about = "registrar turno")]
    Post(PostArgs),
    #[command(about = "listar turnos (default: abertos)")]
    Turns(TurnsArgs),
    #[command(about = "ver turno completo")]
    Show(IdArgs),
    #[command(about = "marcar turno resolvido")]
    Resolve(IdArgs),
    #[command(about = "listar work items (default: pendentes)")]
    Items(ItemsArgs),
    #[command(name = "item-add", about = "criar work item")]
    ItemAdd(ItemAddArgs),
    #[command(name = "item-set", about = "atualizar work item")]
    ItemSet(ItemSetArgs),
    #[command(about = "setar chave de estado (ex: bola_com)")]
    Set(SetArgs),
    #[command(about = "estado atual consolidado")]
    State,
    #[command(about = "listar roster padrão e ad-hoc")]
    Agents,
    #[command(about = "prompt + estado + turnos de um agente")]
    Brief(BriefArgs),
    #[command(name = "agent-add", about = "criar persona ad-hoc")]
    AgentAdd(AgentAddArgs),
    #[command(name = "agent-set", about = "atualizar persona ad-hoc")]
    AgentSet(AgentSetArgs),
    #[command(name = "agent-rm", about = "remover persona ad-hoc")]
    AgentRm(AgentNameArgs),
    #[command(name = "agent-show", about = "imprimir prompt de uma persona")]
    AgentShow(AgentShowArgs),
    #[command(about = "iniciar trabalho em issue do Board")]
    Start(StartArgs),
    #[command(about = "concluir trabalho em issue")]
    Finish(FinishArgs),
    #[command(name = "board-sync", about = "sincronizar GitHub Projects com comms.db")]
    BoardSync,
    #[command(name = "board-item", about = "imprimir item-id e status do card")]
    BoardItem(IssueArgs),
    #[command(name = "board-set", about = "mover card da issue para outra coluna")]
    BoardSet(BoardSetArgs),
    #[command(about = "verificar se toda finding tem destino")]
    Gate(GateArgs),
    #[command(about = "imprimir o branch tronco do repo")]
    Trunk,
    #[command(about = "imprimir o comando de teste do repo")]
    Testcmd,
    #[command(about = "liberar roster em banco antigo")]
    Migrate,
    #[command(about = "inicializar repositório para comunicação de agentes")]
    Bootstrap(BootstrapArgs),
    #[command(about = "exibir versão e configurações")]
    Version,
}

#[derive(Args)]
pub struct PostArgs {
    #[arg(long = "de")]
    pub de: String,
    #[arg(long)]
    pub para: String,
    #[arg(long, value_parser = PossibleValuesParser::new(KINDS))]
    pub tipo: String,
    #[arg(long, allow_hyphen_values = true)]
    pub titulo: String,
    #[arg(long, allow_hyphen_values = true)]
    pub corpo: Option<String>,
    #[arg(long, allow_hyphen_values = true)]
    pub refs: Option<String>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct TurnsArgs {
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub para: Option<String>,
}

#[derive(Args)]
pub struct IdArgs {
    pub id: i64,
}

#[derive(Args)]
pub struct ItemsArgs {
    #[arg(long)]
    pub all: bool,
}

#[derive(Args)]
pub struct ItemAddArgs {
    #[arg(long, allow_hyphen_values = true)]
    pub titulo: String,
    #[arg(long)]
    pub dono: String,
    #[arg(long, default_value = "cross")]
    pub escopo: String,
    #[arg(long, default_value = "aberto", value_parser = PossibleValuesParser::new(STATUSES))]
    pub status: String,
    #[arg(long, allow_hyphen_values = true)]
    pub bloqueio: Option<String>,
    #[arg(long, allow_hyphen_values = true)]
    pub refs: Option<String>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct ItemSetArgs {
    pub id: i64,
    #[arg(long, value_parser = PossibleValuesParser::new(STATUSES))]
    pub status: Option<String>,
    #[arg(long, allow_hyphen_values = true)]
    pub bloqueio: Option<String>,
    #[arg(long, allow_hyphen_values = true)]
    pub titulo: Option<String>,
    #[arg(long)]
    pub dono: Option<String>,
    #[arg(long, allow_hyphen_values = true)]
    pub refs: Option<String>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct SetArgs {
    #[arg(allow_hyphen_values = true)]
    pub key: String,
    #[arg(allow_hyphen_values = true)]
    pub value: String,
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct BriefArgs {
    pub agente: String,
}

#[derive(Args)]
pub struct AgentAddArgs {
    pub nome: String,
    #[arg(long, allow_hyphen_values = true)]
    pub descricao: String,
    #[arg(long)]
    pub por: String,
    #[arg(long = "prompt-file")]
    pub prompt_file: Option<std::path::PathBuf>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct AgentSetArgs {
    pub nome: String,
    #[arg(long, allow_hyphen_values = true)]
    pub descricao: Option<String>,
    #[arg(long = "prompt-file")]
    pub prompt_file: Option<std::path::PathBuf>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct AgentNameArgs {
    pub nome: String,
}

#[derive(Args)]
pub struct AgentShowArgs {
    pub nome: String,
    #[arg(long)]
    pub adhoc: bool,
}

#[derive(Args)]
pub struct GateArgs {
    #[arg(long, allow_hyphen_values = true)]
    pub refs: Option<String>,
}

#[derive(Args)]
pub struct BootstrapArgs {
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct IssueArgs {
    pub issue: u64,
}

#[derive(Args)]
pub struct StartArgs {
    pub issue: u64,
    #[arg(long)]
    pub dono: Option<String>,
    #[arg(long = "no-worktree")]
    pub no_worktree: bool,
    #[arg(long, allow_hyphen_values = true)]
    pub base: Option<String>,
}

#[derive(Args)]
pub struct FinishArgs {
    pub issue: u64,
    #[arg(long, allow_hyphen_values = true)]
    pub pr: String,
}

#[derive(Args)]
pub struct BoardSetArgs {
    pub issue: u64,
    #[arg(allow_hyphen_values = true)]
    pub status: String,
}
