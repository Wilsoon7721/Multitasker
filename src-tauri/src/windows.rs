use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

pub const MAIN: &str = "main";
pub const WIDGET: &str = "widget";

/// Tells every window whether the widget is showing, so toggles stay in step wherever it was changed.
pub const WIDGET_VISIBILITY: &str = "widget-visibility";

/// Gap between the widget and the screen's top-right corner, in logical pixels.
const MARGIN: f64 = 4.0;

fn window(app: &AppHandle, label: &str) -> tauri::Result<WebviewWindow> {
    app.get_webview_window(label).ok_or(tauri::Error::WindowNotFound)
}

pub fn show_main(app: &AppHandle) -> tauri::Result<()> {
    let main = window(app, MAIN)?;
    main.unminimize()?;
    main.show()?;
    main.set_focus()
}

pub fn widget_visible(app: &AppHandle) -> tauri::Result<bool> {
    window(app, WIDGET)?.is_visible()
}

/// Shows the widget pinned top-right, or hides it. Returns whether it is now visible.
pub fn set_widget_visible(app: &AppHandle, visible: bool) -> tauri::Result<bool> {
    let widget = window(app, WIDGET)?;
    if visible {
        pin_top_right(&widget)?;
        widget.show()?;
    } else {
        widget.hide()?;
    }
    let _ = app.emit(WIDGET_VISIBILITY, visible);
    Ok(visible)
}

pub fn toggle_widget(app: &AppHandle) -> tauri::Result<bool> {
    set_widget_visible(app, !widget_visible(app)?)
}

/// Pins the widget to the top-right of the primary monitor's work area, clear of the taskbar.
fn pin_top_right(widget: &WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = widget.primary_monitor()?.or(widget.current_monitor()?) else {
        return Ok(());
    };

    let area = monitor.work_area();
    let margin = (MARGIN * monitor.scale_factor()).round() as i32;
    let width = widget.outer_size()?.width as i32;

    let x = area.position.x + area.size.width as i32 - width - margin;
    let y = area.position.y + margin;
    widget.set_position(PhysicalPosition::new(x, y))
}
