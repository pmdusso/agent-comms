use crate::error::{Code, CommsError, Result};
use crate::policy::HANDOFF_FIELDS;
use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::{UnicodeNormalization, char::canonical_combining_class};

static PII_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+?\d{10,}").unwrap());

pub fn pii_check(texts: &[Option<&str>], force: bool) -> Result<()> {
    if force {
        return Ok(());
    }
    if texts.iter().flatten().any(|text| PII_RE.is_match(text)) {
        return Err(CommsError::new(
            Code::Guard,
            "ERRO: possível PII (sequência de 10+ dígitos). Referencie por ID lógico. Use --force se for falso positivo.",
        ));
    }
    Ok(())
}

pub fn handoff_missing(body: &str) -> Vec<&'static str> {
    let normalized: String = body
        .to_lowercase()
        .nfkd()
        .filter(|c| canonical_combining_class(*c) == 0)
        .collect();
    HANDOFF_FIELDS
        .iter()
        .copied()
        .filter(|field| !normalized.contains(&format!("{field}:")))
        .collect()
}

pub fn refs_has_token(refs: Option<&str>, token: &str) -> bool {
    let Some(refs) = refs else { return false };
    let pattern = format!(r"(?i)(^|[^0-9A-Za-z]){}($|[^0-9A-Za-z])", regex::escape(token));
    Regex::new(&pattern).map(|regex| regex.is_match(refs)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{handoff_missing, pii_check, refs_has_token};

    #[test]
    fn handoff_fields_ignore_case_and_accents() {
        assert_eq!(handoff_missing("objetivo: x\ncriterios: y\nfronteiras: z"), Vec::<&str>::new());
        assert_eq!(handoff_missing("Objetivo: x\nCritérios: y\nFronteiras: z"), Vec::<&str>::new());
        assert_eq!(handoff_missing("Criterios: sem acento\nObjetivo: x"), vec!["fronteiras"]);
        assert_eq!(handoff_missing("vago"), vec!["objetivo", "criterios", "fronteiras"]);
    }

    #[test]
    fn pii_matches_long_digit_sequences_only() {
        assert!(pii_check(&[Some("+5551999998888")], false).is_err());
        assert!(pii_check(&[Some("32165498700")], false).is_err());
        assert!(pii_check(&[Some("activity_id 12345 MVP-153")], false).is_ok());
        assert!(pii_check(&[Some("5551999998888")], true).is_ok());
    }

    #[test]
    fn refs_match_whole_tokens_case_insensitively() {
        assert!(refs_has_token(Some("issue:42 pr:x"), "issue:42"));
        assert!(!refs_has_token(Some("issue:42"), "issue:4"));
        assert!(refs_has_token(Some("issue:4,issue:5"), "issue:4"));
        assert!(refs_has_token(Some("ISSUE:42"), "issue:42"));
        assert!(!refs_has_token(None, "issue:42"));
    }
}
