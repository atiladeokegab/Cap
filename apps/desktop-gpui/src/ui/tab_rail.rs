//! The tab rail: `ConfigSidebar`'s six icon-only triggers under a
//! `KTabs.Indicator` box (`ConfigSidebar.tsx:593-692`).
//!
//! **This is not the settings sidebar's shape.** The usage matrix lists `KTabs`
//! in the editor and the screenshot editor only; the settings window's sidebar
//! is a vertical nav *list* (icon plus label rows, `.cap-settings-nav`), and
//! `settings.tsx` renders it with `<For>` over `settingsItems`, not with tabs.
//! They were checked against each other and kept apart deliberately -- merging
//! them would mean inventing a shared component neither app has.
//!
//! The reference design stands the rail on its side: a vertical icon column down
//! the sidebar card's left edge, with 52x52 tiles.
//!
//! The indicator does not slide (no transform in this gpui rev); the selected
//! item paints the `size-9 rounded-lg bg-gray-3` box itself, which is what the
//! editor window has always done.
//!
//! Each tab is exactly one element -- the pill. A wrapper carrying the hit box,
//! the tooltip anchor or a focus ring would paint its own square corners around
//! the pill's radius, which is what the first pass shipped.

use gpui::{
    App, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px, svg,
};

use crate::theme::Theme;

#[derive(Debug, Clone)]
pub struct TabRailItem {
    pub icon: SharedString,
    /// The tab's name. It is not drawn -- the rail is icon-only -- so it is
    /// carried as the tooltip instead.
    pub label: SharedString,
    pub selected: bool,
    pub disabled: bool,
}

impl TabRailItem {
    pub fn new(
        icon: impl Into<SharedString>,
        label: impl Into<SharedString>,
        selected: bool,
        disabled: bool,
    ) -> Self {
        Self {
            icon: icon.into(),
            label: label.into(),
            selected,
            disabled,
        }
    }
}

/// The index arrives by reference, the same shape the other index-dispatching
/// components use, so `cx.listener` builds the handler directly.
type TabHandler = Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct TabRail {
    id: ElementId,
    items: Vec<TabRailItem>,
    width: Pixels,
    box_size: Pixels,
    icon_size: Pixels,
    bg: Hsla,
    indicator: Hsla,
    hover: Hsla,
    selected_icon: Hsla,
    idle_icon: Hsla,
    theme: Theme,
    on_select: Option<TabHandler>,
}

impl TabRail {
    /// The editor's config sidebar rail, as the reference design draws it: a 60px column on
    /// the card surface, 52x52 tiles with radius 10 and 22px icons.
    pub fn editor(
        theme: &Theme,
        id: impl Into<ElementId>,
        panel_bg: Hsla,
        items: Vec<TabRailItem>,
    ) -> Self {
        Self {
            id: id.into(),
            items,
            width: px(60.),
            box_size: px(52.),
            icon_size: px(22.),
            bg: panel_bg,
            indicator: Hsla::from(theme.editor.rail_tile),
            hover: Hsla::from(theme.editor.ctl),
            selected_icon: Hsla::from(theme.editor.text_1),
            idle_icon: Hsla::from(theme.editor.icon),
            theme: *theme,
            on_select: None,
        }
    }

    /// The rail's width. Kept a parameter so the call site can go on quoting
    /// its own metric.
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    pub fn on_select(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for TabRail {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let TabRail {
            id,
            items,
            width,
            box_size,
            icon_size,
            bg,
            indicator,
            hover,
            selected_icon,
            idle_icon,
            theme,
            on_select,
        } = self;

        let prefix: SharedString = match &id {
            ElementId::Name(name) => name.clone(),
            other => SharedString::from(format!("{other:?}")),
        };
        let handler: Option<std::rc::Rc<TabHandler>> = on_select.map(std::rc::Rc::new);

        div()
            .id(id)
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .w(width)
            .h_full()
            .flex_none()
            .overflow_hidden()
            .gap(px(8.))
            .pt(px(8.))
            .bg(bg)
            .children(items.into_iter().enumerate().map(|(index, item)| {
                let handler = handler.clone();
                let selected = item.selected;
                let disabled = item.disabled;
                let label = item.label.clone();

                div()
                    .id(SharedString::from(format!("{prefix}-{index}")))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(box_size)
                    .rounded(px(crate::theme::look::RADIUS_TILE))
                    .when(disabled, |this| this.opacity(0.6))
                    .when(selected, |this| this.bg(indicator))
                    .when(!selected && !disabled, |this| {
                        this.cursor_pointer().hover(move |style| style.bg(hover))
                    })
                    .tooltip_show_delay(crate::ui::TOOLTIP_SHOW_DELAY)
                    .tooltip(move |_, cx| crate::ui::Tooltip::new(&theme, label.clone()).view(cx))
                    .child(
                        svg()
                            .path(item.icon)
                            .size(icon_size)
                            .text_color(if selected { selected_icon } else { idle_icon }),
                    )
                    .when_some(handler.filter(|_| !disabled), |this, handler| {
                        this.on_click(move |_, window, cx| handler(&index, window, cx))
                    })
            }))
    }
}
