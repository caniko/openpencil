//! Shared font-family catalog used by typography widgets and hosts.

/// TS `BUNDLED_FAMILIES` (use-system-fonts.ts:9-21). The canonical list
/// lives in editor-core because missing-font detection also needs it.
pub use op_editor_core::font_catalog::BUNDLED_FONT_FAMILIES;

/// TS `FALLBACK_SYSTEM_FONTS` (use-system-fonts.ts:24-36).
pub const FALLBACK_SYSTEM_FONTS: [&str; 11] = [
    "Arial",
    "Helvetica",
    "Helvetica Neue",
    "Georgia",
    "Times New Roman",
    "Courier New",
    "Verdana",
    "Trebuchet MS",
    "Tahoma",
    "Impact",
    "Comic Sans MS",
];
