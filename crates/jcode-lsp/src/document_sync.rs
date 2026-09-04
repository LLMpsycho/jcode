use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::json;
use tokio::sync::Mutex;

use crate::{LspClient, LspError, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentState {
    pub path: PathBuf,
    pub uri: String,
    pub language_id: String,
    pub version: i64,
    pub text: String,
}

#[derive(Default)]
pub struct DocumentSync {
    documents: Mutex<HashMap<PathBuf, DocumentState>>,
}

impl DocumentSync {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn sync(
        &self,
        client: &LspClient,
        workspace_root: &Path,
        path: &Path,
        language_id: &str,
        text: String,
        incremental: bool,
    ) -> Result<DocumentState> {
        let path = path.canonicalize()?;
        if !path.starts_with(workspace_root) {
            return Err(LspError::InvalidWorkspaceUri {
                path: path.display().to_string(),
            });
        }
        let uri = file_uri(&path)?;
        let mut documents = self.documents.lock().await;
        if let Some(document) = documents.get_mut(&path) {
            if document.text != text {
                document.version += 1;
                let previous_text = std::mem::replace(&mut document.text, text);
                let content_change = if incremental {
                    json!({
                        "range": full_document_range(&previous_text),
                        "text": document.text
                    })
                } else {
                    json!({"text": document.text})
                };
                client
                    .notify(
                        "textDocument/didChange",
                        Some(json!({
                            "textDocument": {
                                "uri": document.uri,
                                "version": document.version
                            },
                            "contentChanges": [content_change]
                        })),
                    )
                    .await?;
            }
            return Ok(document.clone());
        }

        let document = DocumentState {
            path: path.clone(),
            uri,
            language_id: language_id.to_owned(),
            version: 1,
            text,
        };
        client
            .notify(
                "textDocument/didOpen",
                Some(json!({
                    "textDocument": {
                        "uri": document.uri,
                        "languageId": document.language_id,
                        "version": document.version,
                        "text": document.text
                    }
                })),
            )
            .await?;
        documents.insert(path, document.clone());
        Ok(document)
    }

    pub async fn close(&self, client: &LspClient, path: &Path) -> Result<()> {
        let path = path.canonicalize()?;
        let Some(document) = self.documents.lock().await.remove(&path) else {
            return Ok(());
        };
        client
            .notify(
                "textDocument/didClose",
                Some(json!({"textDocument": {"uri": document.uri}})),
            )
            .await
    }

    pub async fn get(&self, path: &Path) -> Option<DocumentState> {
        let path = path.canonicalize().ok()?;
        self.documents.lock().await.get(&path).cloned()
    }
}

pub(crate) fn file_uri(path: &Path) -> Result<String> {
    url::Url::from_file_path(path)
        .map(|uri| uri.into())
        .map_err(|()| LspError::InvalidWorkspaceUri {
            path: path.display().to_string(),
        })
}

fn full_document_range(text: &str) -> serde_json::Value {
    let mut line = 0_u32;
    let mut character = 0_u32;
    for segment in text.split_inclusive('\n') {
        if segment.ends_with('\n') {
            line += 1;
            character = 0;
        } else {
            character = segment.encode_utf16().count() as u32;
        }
    }
    json!({
        "start": {"line": 0, "character": 0},
        "end": {"line": line, "character": character}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_range_uses_zero_based_utf16_positions() {
        assert_eq!(
            full_document_range("one\n😀x"),
            json!({
                "start": {"line": 0, "character": 0},
                "end": {"line": 1, "character": 3}
            })
        );
        assert_eq!(
            full_document_range("one\n"),
            json!({
                "start": {"line": 0, "character": 0},
                "end": {"line": 1, "character": 0}
            })
        );
    }
}
