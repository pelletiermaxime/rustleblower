//! Bounded content measurement for Kit's native column resize gesture.
use gpui::*;
use gpui_kit::component::table::TableDelegate;

pub(super) fn fit_column<D: TableDelegate>(
    rows: &D,
    index: usize,
    leading_control: bool,
    window: &Window,
    cx: &App,
) -> Pixels {
    let column = rows.column(index, cx);
    let font_size = window.rem_size() * 0.875;
    let style = window.text_style();
    let measure = |text: String| {
        // Shaping remains bounded even for long command lines or paths.
        let text: SharedString = text.chars().take(512).collect::<String>().into();
        window
            .text_system()
            .shape_line(
                text.clone(),
                font_size,
                &[TextRun {
                    len: text.len(),
                    font: style.font(),
                    color: style.color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width
    };
    let sort_icon = if column.sort.is_some() {
        window.rem_size() * 0.75 + px(4.)
    } else {
        px(0.)
    };
    let mut width = measure(column.name.to_string()) + sort_icon;
    for row in 0..rows.rows_count(cx).min(200) {
        let controls = if leading_control {
            // Medium Kit checkbox (1rem) and the cell's gap_2.
            window.rem_size() * 1.5
        } else {
            px(0.)
        };
        width = width.max(measure(rows.cell_text(row, index, cx)) + controls);
        if width >= px(1200.) {
            break;
        }
    }
    width.min(px(1200.))
}
