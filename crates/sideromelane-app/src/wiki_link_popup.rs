//! Wiki-link autocomplete popup widget.
//!
//! Shows a filtered list of note stems when the user opens a `[[` span.
//! The caller owns [`WikiLinkPopup`] state and passes the `TextEdit` response
//! to [`WikiLinkPopup::show`] each frame the popup should be visible.

use eframe::egui;

/// Action returned by [`WikiLinkPopup::show`] when the user clicks a row.
#[derive(Debug, PartialEq, Eq)]
pub enum WikiLinkAction {
    /// User clicked the given note stem.
    Selected(String),
}

/// Persistent autocomplete popup state. Owned by the editor.
#[derive(Debug, Default)]
pub struct WikiLinkPopup {
    items: Vec<String>,
    selected: usize,
}

impl WikiLinkPopup {
    /// Replace the item list, resetting selection to 0 only when the list changes.
    pub(crate) fn set_items(&mut self, items: Vec<String>) {
        if items != self.items {
            self.items = items;
            self.selected = 0;
        }
    }

    /// Move selection to the next item, wrapping from last to first.
    pub(crate) const fn select_next(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1) % self.items.len();
        }
    }

    /// Move selection to the previous item, wrapping from first to last.
    pub(crate) const fn select_prev(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + self.items.len() - 1) % self.items.len();
        }
    }

    /// Return the currently-highlighted stem, or `None` if the list is empty.
    pub(crate) fn selected_item(&self) -> Option<&str> {
        self.items.get(self.selected).map(String::as_str)
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Render the popup anchored below `anchor_response`.
    ///
    /// Handles only click selection — keyboard navigation (Enter/Escape/arrows)
    /// is consumed by the caller before `TextEdit` processes them and is applied
    /// via `raw_popup_pass`.
    /// Returns [`WikiLinkAction::Selected`] on click, `None` otherwise.
    pub(crate) fn show(&self, anchor_response: &egui::Response) -> Option<WikiLinkAction> {
        let popup_id = anchor_response.id.with("wlp");
        let mut clicked: Option<WikiLinkAction> = None;
        egui::Popup::from_response(anchor_response)
            .open(true)
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
            .id(popup_id)
            .show(|ui| {
                ui.set_min_width(200.0);
                for (i, item) in self.items.iter().enumerate() {
                    if ui
                        .selectable_label(i == self.selected, item.as_str())
                        .clicked()
                    {
                        clicked = Some(WikiLinkAction::Selected(item.clone()));
                    }
                }
            });
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn popup_with(items: &[&str]) -> WikiLinkPopup {
        let mut p = WikiLinkPopup::default();
        p.set_items(items.iter().copied().map(String::from).collect());
        p
    }

    #[test]
    fn new_starts_at_first_item() {
        let p = popup_with(&["Alpha", "Beta", "Gamma"]);
        assert_eq!(p.selected_item(), Some("Alpha"));
    }

    #[test]
    fn select_next_advances_selection() {
        let mut p = popup_with(&["Alpha", "Beta", "Gamma"]);
        p.select_next();
        assert_eq!(p.selected_item(), Some("Beta"));
    }

    #[test]
    fn select_next_wraps_from_last_to_first() {
        let mut p = popup_with(&["Alpha", "Beta", "Gamma"]);
        p.select_next();
        p.select_next();
        p.select_next();
        assert_eq!(p.selected_item(), Some("Alpha"));
    }

    #[test]
    fn select_prev_retreats_selection() {
        let mut p = popup_with(&["Alpha", "Beta", "Gamma"]);
        p.select_next();
        p.select_prev();
        assert_eq!(p.selected_item(), Some("Alpha"));
    }

    #[test]
    fn select_prev_wraps_from_first_to_last() {
        let mut p = popup_with(&["Alpha", "Beta", "Gamma"]);
        p.select_prev();
        assert_eq!(p.selected_item(), Some("Gamma"));
    }

    #[test]
    fn selected_item_returns_none_for_empty_list() {
        let p = WikiLinkPopup::default();
        assert_eq!(p.selected_item(), None);
    }

    #[test]
    fn set_items_resets_selection_when_list_changes() {
        let mut p = popup_with(&["Alpha", "Beta", "Gamma"]);
        p.select_next();
        p.set_items(vec!["Delta".into(), "Epsilon".into()]);
        assert_eq!(p.selected, 0);
        assert_eq!(p.selected_item(), Some("Delta"));
    }

    #[test]
    fn set_items_preserves_selection_when_list_unchanged() {
        let mut p = popup_with(&["Alpha", "Beta"]);
        p.select_next();
        p.set_items(vec!["Alpha".into(), "Beta".into()]);
        assert_eq!(
            p.selected, 1,
            "selection must be preserved on identical list"
        );
    }

    #[test]
    fn select_next_on_empty_list_does_not_panic() {
        let mut p = WikiLinkPopup::default();
        p.select_next();
        assert_eq!(p.selected_item(), None);
    }

    #[test]
    fn select_prev_on_empty_list_does_not_panic() {
        let mut p = WikiLinkPopup::default();
        p.select_prev();
        assert_eq!(p.selected_item(), None);
    }
}
