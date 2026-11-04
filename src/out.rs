use crate::error::CommsError;
use serde_json::{Value, json};
use std::io::{ErrorKind, Write};

pub struct Out {
    pub json: bool,
    text: String,
    doc: Option<Value>,
    pub warnings: Vec<String>,
}

impl Out {
    pub fn new(json: bool) -> Self {
        Self { json, text: String::new(), doc: None, warnings: Vec::new() }
    }

    pub fn say(&mut self, line: impl AsRef<str>) {
        if !self.json {
            self.text.push_str(line.as_ref());
            self.text.push('\n');
        }
    }

    pub fn raw(&mut self, text: &str) {
        if !self.json {
            self.text.push_str(text);
            if !text.ends_with('\n') {
                self.text.push('\n');
            }
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.json {
            self.warnings.push(message);
        } else {
            self.say(message);
        }
    }

    pub fn set(&mut self, doc: Value) {
        self.doc = Some(doc);
    }

    pub fn flush(&self) -> std::io::Result<()> {
        self.flush_to(&mut std::io::stdout().lock())
    }

    fn flush_to(&self, writer: &mut dyn Write) -> std::io::Result<()> {
        let result = if self.json {
            let mut doc = self.doc.clone().unwrap_or_else(|| json!({}));
            if !self.warnings.is_empty() {
                doc["warnings"] = json!(self.warnings);
            }
            let mut bytes = serde_json::to_vec(&doc).unwrap_or_else(|_| b"{}".to_vec());
            bytes.push(b'\n');
            writer.write_all(&bytes)
        } else {
            writer.write_all(self.text.as_bytes())
        };
        match result {
            Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(()),
            result => result,
        }
    }
}

pub fn emit_error(json_mode: bool, error: &CommsError, warnings: &[String]) {
    if json_mode {
        let mut doc = json!({
            "error": {
                "code": error.code.as_str(),
                "message": error.message,
            }
        });
        if !warnings.is_empty() {
            doc["warnings"] = json!(warnings);
        }
        let mut bytes = serde_json::to_vec(&doc).unwrap_or_else(|_| "{\"error\":{\"code\":\"internal\",\"message\":\"ERRO de serialização\"}}".as_bytes().to_vec());
        bytes.push(b'\n');
        let _ = std::io::stderr().lock().write_all(&bytes);
    } else {
        let _ = writeln!(std::io::stderr().lock(), "{}", error.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    struct FailingWriter(ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "falha de teste"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn flush_to_distinguishes_broken_pipe_from_real_errors() {
        let mut out = Out::new(false);
        out.say("conteúdo");
        assert_eq!(out.flush_to(&mut FailingWriter(ErrorKind::BrokenPipe)).unwrap(), ());
        let error = out.flush_to(&mut FailingWriter(ErrorKind::WriteZero)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::WriteZero);
    }
}
