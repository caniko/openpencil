//! The ready-view history list's expanded inline detail card
//! (里程碑详情 — semantic diff summary + patch list + 恢复/复制哈希
//! buttons). Split out of `git_panel_ready.rs` to keep that file
//! under the repo's 800-line cap.
//!
//! The card paints as a proper inset panel (jian `Card` — a muted
//! fill plus a border at a radius, the same primitive
//! `agent_settings_panel_card` uses for provider cards) rather than a
//! full-width flush band, for two reasons: it reads as a distinct
//! "detail" surface instead of bleeding into the timeline flow, AND —
//! since a full-width band would paint directly over the timeline
//! rail's dot column — insetting it past the rail keeps the vertical
//! connector line visible underneath the expanded card instead of
//! visually severing it.

use crate::widgets::git_panel::{GitPanel, GitPanelHit};
use crate::widgets::git_panel_ready::{MAX_COMMITS, MSG_X, READY_PAD, ROW_H};
use crate::widgets::PaintCx;
use crate::{Color, Point2D, Rect};
use jian_widgets::components::card::Card;
use op_editor_core::CommitDiffSummary;
use op_editor_core::CommitDiffView;

/// Base height of the inline commit-detail card (里程碑详情 title +
/// one status/summary line + a restore/copy-hash button row) under
/// an expanded commit row (TS `HistoryMilestoneRow` detail block).
/// Grows by one [`PATCH_ROW_H`] per rendered patch line. The extra
/// height over the button row (anchored at `height - 52`) is bottom
/// padding so the next commit row stays well clear of the buttons.
const CARD_BASE_H: f32 = 104.0;
/// Per-patch-line height in the expanded diff list.
const PATCH_ROW_H: f32 = 14.0;
/// Cap on patch lines drawn in the card (TS scrolls a `max-h-24`
/// list; the summary counts still report the full totals above the
/// list).
const MAX_PATCH_ROWS: usize = 6;

impl GitPanel<'_> {
    /// Number of patch lines the expanded card will draw (0 unless
    /// the diff is `Ready`, capped at [`MAX_PATCH_ROWS`]).
    fn card_patch_rows(&self) -> usize {
        match &self.state.expanded_commit_diff {
            Some(CommitDiffView::Ready(s)) => s.patches.len().min(MAX_PATCH_ROWS),
            _ => 0,
        }
    }

    /// Total height of the open card — base chrome plus one row per
    /// drawn patch line. Shared by paint + the hit-test walk so they
    /// agree.
    pub(super) fn expanded_card_height(&self) -> f32 {
        CARD_BASE_H + self.card_patch_rows() as f32 * PATCH_ROW_H
    }

    /// Extra height contributed by an open inline commit-detail card
    /// — [`Self::expanded_card_height`] when a valid row is expanded,
    /// else 0. Shared by [`GitPanel::ready_height`] + the history
    /// paint / hit-test walk so they stay in lockstep.
    pub(super) fn expanded_card_extra(&self) -> f32 {
        let n = self.state.recent_commits.len().min(MAX_COMMITS);
        match self.state.expanded_commit {
            Some(e) if e < n => self.expanded_card_height(),
            _ => 0.0,
        }
    }

    /// Vertical offset inserted before commit row `i` by a detail
    /// card open under an earlier row.
    pub(super) fn expand_offset_before(&self, i: usize) -> f32 {
        let n = self.state.recent_commits.len().min(MAX_COMMITS);
        match self.state.expanded_commit {
            Some(e) if e < i && e < n => self.expanded_card_height(),
            _ => 0.0,
        }
    }

    /// The inline detail card's own rect at `card_top` — inset so
    /// its left edge clears the timeline rail + dot column (see the
    /// module doc). Shared by paint + the button-rect geometry so
    /// they can never drift apart.
    fn card_rect_at(&self, rect: Rect, card_top: f32) -> Rect {
        let card_x = rect.origin.x + MSG_X - 8.0;
        Rect {
            origin: Point2D::new(card_x, card_top),
            size: Point2D::new(
                (rect.origin.x + rect.size.x - READY_PAD - card_x).max(0.0),
                self.expanded_card_height(),
            ),
        }
    }

    /// `(恢复, 复制哈希)` button rects for the inline card whose top
    /// edge is `card_top`. Backend-free fixed widths keep paint +
    /// hit aligned. The button row is bottom-anchored so a growing
    /// patch list pushes it down in lockstep with paint.
    fn commit_card_button_rects(&self, rect: Rect, card_top: f32) -> (Rect, Rect) {
        // Button row sits at content position `52 + patches`; the rest of the
        // card height (CARD_BASE_H - 52 = 52px) is bottom padding so the next
        // commit row keeps well clear of the buttons.
        let card = self.card_rect_at(rect, card_top);
        let btn_y = card_top + self.expanded_card_height() - 52.0;
        let h = 24.0;
        let x = card.origin.x + 12.0;
        let restore = Rect {
            origin: Point2D::new(x, btn_y),
            size: Point2D::new(64.0, h),
        };
        let copy = Rect {
            origin: Point2D::new(x + 64.0 + 8.0, btn_y),
            size: Point2D::new(84.0, h),
        };
        (restore, copy)
    }

    /// `(恢复, 复制哈希)` rects for the currently-expanded card, or
    /// `None` when nothing is expanded. Mirrors the history paint
    /// y-walk so the hit-test lands exactly where paint drew the
    /// buttons.
    pub(super) fn ready_commit_card_buttons(&self, rect: Rect) -> Option<(Rect, Rect)> {
        let n = self.state.recent_commits.len().min(MAX_COMMITS);
        let e = self.state.expanded_commit.filter(|&e| e < n)?;
        // Row `e`'s text baseline (no prior card offsets — only one card
        // can be open) then `+ ROW_H` to the card top, matching paint.
        let card_top = rect.origin.y + self.history_first() + (e as f32 + 1.0) * ROW_H - 6.0;
        Some(self.commit_card_button_rects(rect, card_top))
    }

    /// Paint the inline commit-detail card (里程碑详情) — an inset
    /// card (see the module doc) holding the detail title, the
    /// semantic diff (TS `GitPanelHistoryDiff`: a summary row + an
    /// `op nodeId` patch list, or a loading / initial / no-changes /
    /// error line), and a `恢复` / `复制哈希` button row.
    pub(super) fn paint_commit_card(&self, cx: &mut PaintCx<'_>, rect: Rect, card_top: f32) {
        let t = self.theme;
        let card = self.card_rect_at(rect, card_top);
        Card {
            fill: Some(alpha(t.muted, 0.55)),
            border: Some(alpha(t.border, 0.60)),
            radius: 8.0,
        }
        .paint(
            cx.backend,
            card,
            &crate::widgets::button::tokens_from_theme(&t),
        );
        let body_x = card.origin.x + 12.0;
        // Title — `text-[11px] font-medium`.
        self.text(
            cx,
            self.t("git.history.milestoneDetailTitle"),
            body_x,
            card_top + 18.0,
            11.0,
            t.foreground,
        );
        // Diff body (TS `GitPanelHistoryDiff` states).
        let status_y = card_top + 38.0;
        match &self.state.expanded_commit_diff {
            None | Some(CommitDiffView::Loading) => {
                self.text(
                    cx,
                    self.t("git.history.diff.loading"),
                    body_x,
                    status_y,
                    10.0,
                    alpha(t.muted_foreground, 0.85),
                );
            }
            Some(CommitDiffView::Initial) => {
                self.text(
                    cx,
                    self.t("git.history.diff.initialCommit"),
                    body_x,
                    status_y,
                    10.0,
                    alpha(t.muted_foreground, 0.85),
                );
            }
            Some(CommitDiffView::NoChanges) => {
                self.text(
                    cx,
                    self.t("git.history.diff.noChanges"),
                    body_x,
                    status_y,
                    10.0,
                    alpha(t.muted_foreground, 0.85),
                );
            }
            Some(CommitDiffView::Error(msg)) => {
                let label = self.t("git.history.diff.error").replace("{{message}}", msg);
                self.text(cx, &label, body_x, status_y, 10.0, t.destructive);
            }
            Some(CommitDiffView::Ready(summary)) => {
                self.paint_diff_summary(cx, summary, body_x, status_y);
                // Patch list — one `op nodeId` line each (TS font-mono).
                for (k, p) in summary.patches.iter().take(MAX_PATCH_ROWS).enumerate() {
                    let py = card_top + 54.0 + k as f32 * PATCH_ROW_H;
                    self.text(cx, &p.op, body_x, py, 10.0, t.foreground);
                    let opw = cx.backend.measure_text(&p.op, 10.0);
                    self.text(
                        cx,
                        &p.node_id,
                        body_x + opw + 6.0,
                        py,
                        10.0,
                        alpha(t.muted_foreground, 0.70),
                    );
                }
            }
        }
        let (restore, copy) = self.commit_card_button_rects(rect, card_top);
        if let Some(e) = self.state.expanded_commit {
            // Restore / Copy hash share the same secondary-button look
            // (muted fill + border + hover wash) as every other
            // non-primary button in the panel — a bordered "Restore"
            // next to a bare-text "Copy hash" used to read as two
            // different affordances.
            self.paint_button_with_hit(
                cx,
                restore,
                self.t("git.history.restoreButton"),
                true,
                false,
                Some(GitPanelHit::RestoreCommit(e)),
            );
            self.paint_button_with_hit(
                cx,
                copy,
                self.t("git.history.copyHashButton"),
                true,
                false,
                Some(GitPanelHit::CopyCommitHash(e)),
            );
        }
    }

    /// Paint the diff summary row — coloured `framesChanged` / `+added` /
    /// `-removed` / `~modified` segments left-to-right (TS `GitPanelHistoryDiff`
    /// summary spans). Only non-zero counts render.
    fn paint_diff_summary(&self, cx: &mut PaintCx<'_>, s: &CommitDiffSummary, x: f32, y: f32) {
        let t = self.theme;
        // Build the (label, colour) segments first so the draw loop borrows
        // `self` only through `self.text` / the backend measure.
        let mut segments: Vec<(String, Color)> = Vec::new();
        if s.frames_changed > 0 {
            segments.push((
                self.plural(
                    "git.history.diff.framesChanged_one",
                    "git.history.diff.framesChanged_other",
                    s.frames_changed,
                ),
                t.muted_foreground,
            ));
        }
        if s.nodes_added > 0 {
            segments.push((
                format!(
                    "+{}",
                    self.plural(
                        "git.history.diff.nodesAdded_one",
                        "git.history.diff.nodesAdded_other",
                        s.nodes_added,
                    )
                ),
                t.primary,
            ));
        }
        if s.nodes_removed > 0 {
            segments.push((
                format!(
                    "-{}",
                    self.plural(
                        "git.history.diff.nodesRemoved_one",
                        "git.history.diff.nodesRemoved_other",
                        s.nodes_removed,
                    )
                ),
                t.destructive,
            ));
        }
        if s.nodes_modified > 0 {
            segments.push((
                format!(
                    "~{}",
                    self.plural(
                        "git.history.diff.nodesModified_one",
                        "git.history.diff.nodesModified_other",
                        s.nodes_modified,
                    )
                ),
                t.muted_foreground,
            ));
        }
        let mut cur = x;
        for (label, color) in &segments {
            self.text(cx, label, cur, y, 10.0, *color);
            cur += cx.backend.measure_text(label, 10.0) + 10.0;
        }
    }

    /// Pick the `_one` / `_other` plural form (TS i18next English rule: 1 →
    /// one) and substitute `{{count}}`. Both keys are `&'static` so they
    /// satisfy [`GitPanel::t`]'s static-key contract.
    fn plural(&self, one_key: &'static str, other_key: &'static str, count: u32) -> String {
        let key = if count == 1 { one_key } else { other_key };
        self.t(key).replace("{{count}}", &count.to_string())
    }
}

/// A colour at `factor` of its current alpha (Tailwind `/NN`).
fn alpha(c: Color, factor: f32) -> Color {
    crate::util::alpha(c, factor)
}
