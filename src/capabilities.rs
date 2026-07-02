//! Position-encoding negotiation and the server's advertised capabilities.
//!
//! Everything advertised here must be answerable — clients are entitled to
//! call any capability we claim. Stub handlers exist from M0 on; features
//! replace them milestone by milestone.

use lsp_types::{
    ClientCapabilities, CodeActionKind, CodeActionOptions, CodeActionProviderCapability,
    ExecuteCommandOptions, OneOf, PositionEncodingKind, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
};

use crate::position::PositionEncoding;

/// The one command the server executes on itself: re-parse a document under
/// a different dialect (`arguments: [uri, "csv"|"tsv"|"ssv"]`). Carried by
/// the `Reinterpret as …` code actions.
pub const SET_DIALECT_COMMAND: &str = "csv-lsp.setDialect";

/// Pick the position encoding from the client's offer: prefer `utf-8` (our
/// offsets are bytes already), then `utf-32`, falling back to the protocol's
/// mandatory `utf-16`.
pub fn negotiate_position_encoding(client: &ClientCapabilities) -> PositionEncoding {
    let offered = client
        .general
        .as_ref()
        .and_then(|general| general.position_encodings.as_deref())
        .unwrap_or(&[]);
    if offered.contains(&PositionEncodingKind::UTF8) {
        PositionEncoding::Utf8
    } else if offered.contains(&PositionEncodingKind::UTF32) {
        PositionEncoding::Utf32
    } else {
        PositionEncoding::Utf16
    }
}

/// The wire constant for a negotiated encoding.
pub fn encoding_kind(enc: PositionEncoding) -> PositionEncodingKind {
    match enc {
        PositionEncoding::Utf8 => PositionEncodingKind::UTF8,
        PositionEncoding::Utf16 => PositionEncodingKind::UTF16,
        PositionEncoding::Utf32 => PositionEncodingKind::UTF32,
    }
}

/// Build the capabilities advertised in the `initialize` response.
pub fn server_capabilities(enc: PositionEncoding) -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(encoding_kind(enc)),
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                ..Default::default()
            },
        )),
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![
                CodeActionKind::QUICKFIX,
                CodeActionKind::SOURCE,
                CodeActionKind::SOURCE_FIX_ALL,
                CodeActionKind::REFACTOR,
                CodeActionKind::REFACTOR_REWRITE,
            ]),
            // Edits are cheap for CSV: computed eagerly, no lazy resolve.
            resolve_provider: Some(false),
            ..Default::default()
        })),
        document_formatting_provider: Some(OneOf::Left(true)),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec![SET_DIALECT_COMMAND.to_owned()],
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_offering(encodings: &[PositionEncodingKind]) -> ClientCapabilities {
        ClientCapabilities {
            general: Some(lsp_types::GeneralClientCapabilities {
                position_encodings: Some(encodings.to_vec()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn utf8_is_preferred_when_offered() {
        let client = client_offering(&[PositionEncodingKind::UTF16, PositionEncodingKind::UTF8]);
        assert_eq!(negotiate_position_encoding(&client), PositionEncoding::Utf8);
    }

    #[test]
    fn utf32_beats_utf16() {
        let client = client_offering(&[PositionEncodingKind::UTF32, PositionEncodingKind::UTF16]);
        assert_eq!(
            negotiate_position_encoding(&client),
            PositionEncoding::Utf32
        );
    }

    #[test]
    fn utf16_is_the_mandatory_default() {
        assert_eq!(
            negotiate_position_encoding(&ClientCapabilities::default()),
            PositionEncoding::Utf16
        );
        assert_eq!(
            negotiate_position_encoding(&client_offering(&[])),
            PositionEncoding::Utf16
        );
    }

    #[test]
    fn capabilities_advertise_sync_actions_and_formatting() {
        let caps = server_capabilities(PositionEncoding::Utf8);
        assert_eq!(caps.position_encoding, Some(PositionEncodingKind::UTF8));
        assert_eq!(caps.document_formatting_provider, Some(OneOf::Left(true)));

        let Some(TextDocumentSyncCapability::Options(sync)) = caps.text_document_sync else {
            panic!("expected sync options");
        };
        assert_eq!(sync.open_close, Some(true));
        assert_eq!(sync.change, Some(TextDocumentSyncKind::FULL));

        let Some(CodeActionProviderCapability::Options(actions)) = caps.code_action_provider else {
            panic!("expected code action options");
        };
        let kinds = actions.code_action_kinds.unwrap();
        assert!(kinds.contains(&CodeActionKind::QUICKFIX));
        assert!(kinds.contains(&CodeActionKind::SOURCE));
        assert!(kinds.contains(&CodeActionKind::SOURCE_FIX_ALL));
        assert!(kinds.contains(&CodeActionKind::REFACTOR));
        assert!(kinds.contains(&CodeActionKind::REFACTOR_REWRITE));
        assert_eq!(actions.resolve_provider, Some(false));
    }

    #[test]
    fn the_set_dialect_command_is_advertised() {
        let caps = server_capabilities(PositionEncoding::Utf8);
        let commands = caps.execute_command_provider.unwrap().commands;
        assert_eq!(commands, [SET_DIALECT_COMMAND]);
    }
}
