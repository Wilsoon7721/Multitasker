use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow};

pub const MAIN: &str = "main";
pub const WIDGET: &str = "widget";

/// Gap between the widget and the screen's top-right corner, in logical pixels.
const MARGIN: f64 = 12.0;

fn window(app: &AppHandle, label: &str) -> tauri::Result<WebviewWindow> {
    app.get_webview_window(label).ok_or(tauri::Error::WindowNotFound)
}

pub fn show_main(app: &AppHandle) -> tauri::Result<()> {
    let main = window(app, MAIN)?;
    main.unminimize()?;
    main.show()?;
    main.set_focus()
}

/// Pins the widget to the top-right of the primary monitor's work area (clear of the taskbar), then shows it.
pub fn show_widget(app: &AppHandle) -> tauri::Result<()> {
    let widget = window(app, WIDGET)?;
    pin_top_right(&widget)?;
    widget.show()
}

/// Shows or hides the widget; returns whether it is now visible.
pub fn toggle_widget(app: &AppHandle) -> tauri::Result<bool> {
    let widget = window(app, WIDGET)?;
    if widget.is_visible()? {
        widget.hide()?;
        Ok(false)
    } else {
        show_widget(app)?;
        Ok(true)
    }
}

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
