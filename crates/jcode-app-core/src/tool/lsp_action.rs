use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LspAction {
    Status,
    Diagnostics,
    Hover,
    Definition,
    References,
    DocumentSymbols,
    WorkspaceSymbols,
    Capabilities,
    Rename,
    RenameFile,
    Implementation,
    TypeDefinition,
    SignatureHelp,
    IncomingCalls,
    OutgoingCalls,
    CodeActions,
    Reload,
}

impl LspAction {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Diagnostics => "diagnostics",
            Self::Hover => "hover",
            Self::Definition => "definition",
            Self::References => "references",
            Self::DocumentSymbols => "document_symbols",
            Self::WorkspaceSymbols => "workspace_symbols",
            Self::Capabilities => "capabilities",
            Self::Rename => "rename",
            Self::RenameFile => "rename_file",
            Self::Implementation => "implementation",
            Self::TypeDefinition => "type_definition",
            Self::SignatureHelp => "signature_help",
            Self::IncomingCalls => "incoming_calls",
            Self::OutgoingCalls => "outgoing_calls",
            Self::CodeActions => "code_actions",
            Self::Reload => "reload",
        }
    }
}
