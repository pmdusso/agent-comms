use std::fmt::{Display, Formatter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Code {
    Usage,
    Invalid,
    NotFound,
    DbMissing,
    Guard,
    Conflict,
    External,
    Internal,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Invalid => "invalid",
            Self::NotFound => "not_found",
            Self::DbMissing => "db_missing",
            Self::Guard => "guard",
            Self::Conflict => "conflict",
            Self::External => "external",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug)]
pub struct CommsError {
    pub code: Code,
    pub message: String,
}

impl CommsError {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn exit_code(&self) -> u8 {
        if self.code == Code::Usage { 2 } else { 1 }
    }
}

impl Display for CommsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommsError {}

impl From<rusqlite::Error> for CommsError {
    fn from(error: rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::SqliteFailure(code, detail) => {
                let detail = detail.unwrap_or_else(|| code.to_string());
                match code.extended_code {
                    rusqlite::ffi::SQLITE_CONSTRAINT_CHECK => Self::new(
                        Code::Conflict,
                        format!("ERRO: o banco rejeitou a escrita ({detail}). Banco criado em versão <= 3 congela o roster: rode 'comms migrate'."),
                    ),
                    rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                    | rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE => Self::new(
                        Code::Conflict,
                        format!("ERRO: registro já existe ({detail})."),
                    ),
                    _ => Self::new(Code::Internal, format!("ERRO SQLite: {code}: {detail}")),
                }
            }
            other => Self::new(Code::Internal, format!("ERRO SQLite: {other}")),
        }
    }
}

impl From<std::io::Error> for CommsError {
    fn from(error: std::io::Error) -> Self {
        Self::new(Code::Internal, format!("ERRO de E/S: {error}"))
    }
}

pub type Result<T, E = CommsError> = std::result::Result<T, E>;
