//! Shared Figma / HTML import-progress overlay. A 360×140 centred card
//! selects its glyph and copy from [`ImportSource`], followed by an animated
//! dot spinner and source-specific subtitle. The parent paints the scrim.
//!
//! The spinner is driven by `now_ms` (passed in from the host's
//! animation clock); native import-session pumps and the web repaint loop keep
//! scheduling frames while work is pending.

use crate::theme::Theme;
use crate::widgets::editor_state_ext::theme_for;
use crate::widgets::icons::{draw_icon, Icon};
use crate::widgets::{LayoutBox, LayoutCx, PaintCx, Widget, WidgetId};
use crate::{Point2D, Rect, TextLayout};
use op_editor_core::editor_ui_state::Locale;
use op_editor_core::figma_import_state::ImportSource;
use op_editor_core::EditorState;

const CARD_WIDTH: f32 = 360.0;
const CARD_HEIGHT: f32 = 140.0;

pub struct ImportProgressOverlay {
    pub id: WidgetId,
    pub theme: Theme,
    locale: Locale,
    source: ImportSource,
    now_ms: u64,
}

impl ImportProgressOverlay {
    pub fn for_editor(state: &EditorState, now_ms: u64) -> Self {
        Self {
            id: WidgetId::new(5450),
            theme: theme_for(&state.editor_ui),
            locale: state.editor_ui.locale,
            source: state.editor_ui.import_source,
            now_ms,
        }
    }

    pub fn rect(&self, viewport_w: f32, viewport_h: f32) -> Rect {
        let x = ((viewport_w - CARD_WIDTH) / 2.0).max(16.0);
        let y = ((viewport_h - CARD_HEIGHT) / 2.0).max(crate::widgets::TOP_BAR_HEIGHT + 16.0);
        Rect {
            origin: Point2D::new(x, y),
            size: Point2D::new(CARD_WIDTH, CARD_HEIGHT),
        }
    }
}

/// Headline + subtitle fallbacks stay local until the canonical locale tables
/// gain source-specific progress keys.
fn parsing_title(locale: Locale, source: ImportSource) -> &'static str {
    match (source, locale) {
        (ImportSource::Figma, Locale::ZhCn) => "正在解析 Figma 文件…",
        (ImportSource::Figma, Locale::ZhTw) => "正在解析 Figma 檔案…",
        (ImportSource::Figma, Locale::Ja) => "Figma ファイルを解析しています…",
        (ImportSource::Figma, Locale::Ko) => "Figma 파일 분석 중…",
        (ImportSource::Figma, _) => "Parsing Figma file…",
        (ImportSource::Html, Locale::ZhCn) => "正在解析 HTML 和页面资源…",
        (ImportSource::Html, Locale::ZhTw) => "正在解析 HTML 和頁面資源…",
        (ImportSource::Html, Locale::Ja) => "HTML とページ素材を解析しています…",
        (ImportSource::Html, Locale::Ko) => "HTML 및 페이지 리소스 분석 중…",
        (ImportSource::Html, _) => "Parsing HTML and page resources…",
    }
}

fn parsing_subtitle(locale: Locale, source: ImportSource) -> &'static str {
    match (source, locale) {
        (ImportSource::Html, Locale::ZhCn) => "正在读取样式和图片，请稍候",
        (ImportSource::Html, Locale::ZhTw) => "正在讀取樣式和圖片，請稍候",
        (ImportSource::Html, Locale::Ja) => "スタイルと画像を読み込んでいます。",
        (ImportSource::Html, Locale::Ko) => "스타일과 이미지를 불러오는 중입니다.",
        (ImportSource::Html, _) => "Loading styles and images. Please wait.",
        (_, Locale::ZhCn) => "大型文件需要几秒钟，请稍候",
        (_, Locale::ZhTw) => "大型檔案需要幾秒，請稍候",
        (_, Locale::Ja) => "大きなファイルは数秒かかります。お待ちください。",
        (_, Locale::Ko) => "큰 파일은 몇 초가 걸립니다. 잠시만 기다려 주세요.",
        _ => "Large files take a few seconds. Please wait.",
    }
}

impl Widget for ImportProgressOverlay {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn layout(&self, _cx: &LayoutCx) -> LayoutBox {
        LayoutBox {
            rect: Rect {
                origin: Point2D::new(0.0, 0.0),
                size: Point2D::new(CARD_WIDTH, CARD_HEIGHT),
            },
        }
    }

    fn paint(&self, cx: &mut PaintCx<'_>, rect: Rect) {
        cx.backend.fill_round_rect(rect, 12.0, self.theme.card);
        cx.backend
            .stroke_round_rect(rect, 12.0, self.theme.border, 1.0);

        // Source glyph, top-centre of the card.
        let glyph_size = 32.0;
        let glyph_x = rect.origin.x + rect.size.x / 2.0 - glyph_size / 2.0;
        let glyph_y = rect.origin.y + 16.0;
        match self.source {
            ImportSource::Figma => crate::widgets::brand_icons::paint_figma_logo(
                cx.backend,
                Point2D::new(glyph_x, glyph_y),
                glyph_size,
                self.theme.muted_foreground,
            ),
            ImportSource::Html => draw_icon(
                cx.backend,
                Icon::FileText,
                Point2D::new(glyph_x, glyph_y),
                glyph_size,
                self.theme.muted_foreground,
                1.8,
            ),
        }

        // Headline directly below the glyph.
        let headline = parsing_title(self.locale, self.source);
        let head_w = cx.backend.measure_text(headline, 14.0);
        let head_layout = TextLayout::single_run(
            headline,
            "system-ui",
            14.0,
            (self.theme.foreground).to_jian(),
            Point2D::new(0.0, 0.0),
        );
        cx.backend.draw_text(
            &head_layout,
            Point2D::new(
                rect.origin.x + (rect.size.x - head_w) / 2.0,
                glyph_y + glyph_size + 22.0,
            ),
        );

        // Animated dot spinner — three dots ticking in a 750 ms cycle.
        // The active dot pops to `foreground`; idle dots sit at
        // `muted_foreground`.
        let dots_y = glyph_y + glyph_size + 42.0;
        let dot_r = 3.5;
        let dot_gap = 12.0;
        let dot_count = 3.0;
        let dots_w = dot_count * dot_r * 2.0 + (dot_count - 1.0) * dot_gap;
        let dots_x = rect.origin.x + (rect.size.x - dots_w) / 2.0;
        let active = ((self.now_ms / 250) % 3) as i32;
        for i in 0..3 {
            let cx_dot = dots_x + dot_r + i as f32 * (dot_r * 2.0 + dot_gap);
            let color = if i == active {
                self.theme.foreground
            } else {
                self.theme.muted_foreground
            };
            cx.backend.fill_oval(
                Rect {
                    origin: Point2D::new(cx_dot - dot_r, dots_y),
                    size: Point2D::new(dot_r * 2.0, dot_r * 2.0),
                },
                color,
            );
        }

        // Subtitle below the dots.
        let sub = parsing_subtitle(self.locale, self.source);
        let sub_w = cx.backend.measure_text(sub, 11.0);
        let sub_layout = TextLayout::single_run(
            sub,
            "system-ui",
            11.0,
            (self.theme.muted_foreground).to_jian(),
            Point2D::new(0.0, 0.0),
        );
        cx.backend.draw_text(
            &sub_layout,
            Point2D::new(rect.origin.x + (rect.size.x - sub_w) / 2.0, dots_y + 22.0),
        );
    }

    fn access_node(&self) -> accesskit::Node {
        let mut node = accesskit::Node::new(accesskit::Role::Dialog);
        node.set_label(parsing_title(self.locale, self.source));
        node.set_busy();
        node
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_progress_copy_is_source_specific() {
        let title = parsing_title(Locale::ZhCn, ImportSource::Html);
        assert!(title.contains("HTML"));
        assert!(!title.contains("Figma"));
        assert!(parsing_subtitle(Locale::EnUs, ImportSource::Html).contains("styles"));
    }
}
