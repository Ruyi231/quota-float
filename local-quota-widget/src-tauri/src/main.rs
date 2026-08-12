#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod quota;
mod skin;

use std::sync::{Mutex, MutexGuard};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, LogicalSize, Manager, PhysicalPosition, State, WebviewWindow, WindowEvent,
};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

const COLLAPSED_SIZE: f64 = 80.0;
const EXPANDED_SIZE: f64 = 356.0;

#[derive(Default)]
struct WindowGeometry {
    collapsed_anchor: Option<PhysicalPosition<i32>>,
    expanded: bool,
    expanded_dragged: bool,
    programmatic_expanded_position: Option<PhysicalPosition<i32>>,
}

impl WindowGeometry {
    fn begin_expand(
        &mut self,
        collapsed_anchor: PhysicalPosition<i32>,
        expanded_position: PhysicalPosition<i32>,
    ) {
        self.collapsed_anchor = Some(collapsed_anchor);
        self.expanded = true;
        self.expanded_dragged = false;
        self.programmatic_expanded_position =
            (expanded_position != collapsed_anchor).then_some(expanded_position);
    }

    fn record_move(&mut self, position: PhysicalPosition<i32>) {
        if !self.expanded {
            return;
        }
        if self.programmatic_expanded_position == Some(position) {
            return;
        }
        if self.collapsed_anchor != Some(position) {
            self.expanded_dragged = true;
            self.programmatic_expanded_position = None;
        }
    }

    fn mark_expanded_drag(&mut self) {
        if self.expanded {
            self.expanded_dragged = true;
            self.programmatic_expanded_position = None;
        }
    }

    fn begin_collapse(&mut self) -> (bool, Option<PhysicalPosition<i32>>) {
        let should_restore_anchor = !self.expanded_dragged;
        let anchor = self.collapsed_anchor;
        self.expanded = false;
        self.expanded_dragged = false;
        self.programmatic_expanded_position = None;
        (should_restore_anchor, anchor)
    }
}

#[derive(Default)]
struct WindowState {
    geometry: Mutex<WindowGeometry>,
}

fn recover_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("widget") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示浮窗", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "彻底退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Codex 本地额度");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "quit" => {
                let _ = app.save_window_state(StateFlags::POSITION);
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[tauri::command]
async fn get_quota_snapshot() -> quota::QuotaSnapshot {
    match tauri::async_runtime::spawn_blocking(quota::read_latest_snapshot).await {
        Ok(snapshot) => snapshot,
        Err(_) => quota::QuotaSnapshot::missing("读取本地额度记录时发生内部错误。"),
    }
}

#[tauri::command]
async fn get_quota_snapshot_online() -> Result<quota::QuotaSnapshot, String> {
    quota::read_quota_online().await
}

#[tauri::command]
async fn get_reset_credits_online() -> Result<quota::ResetCreditSnapshot, String> {
    quota::read_reset_credits_online().await
}

#[tauri::command]
fn resize_widget(
    expanded: bool,
    window: WebviewWindow,
    state: State<'_, WindowState>,
) -> Result<(), String> {
    let current_position = window
        .outer_position()
        .map_err(|error| format!("无法读取窗口位置：{error}"))?;

    if expanded {
        let monitor = window
            .current_monitor()
            .map_err(|error| format!("无法读取显示器信息：{error}"))?;
        let scale = monitor
            .as_ref()
            .map(|item| item.scale_factor())
            .unwrap_or(1.0);
        let target = (EXPANDED_SIZE * scale).round() as i32;
        let mut next = current_position;
        if let Some(monitor) = monitor {
            let bounds_position = *monitor.position();
            let bounds_size = *monitor.size();
            let right = bounds_position.x + bounds_size.width as i32;
            let bottom = bounds_position.y + bounds_size.height as i32;
            next.x = next
                .x
                .clamp(bounds_position.x, (right - target).max(bounds_position.x));
            next.y = next
                .y
                .clamp(bounds_position.y, (bottom - target).max(bounds_position.y));
        }

        recover_lock(&state.geometry).begin_expand(current_position, next);
        window
            .set_position(next)
            .map_err(|error| format!("无法调整窗口位置：{error}"))?;
        window
            .set_size(LogicalSize::new(EXPANDED_SIZE, EXPANDED_SIZE))
            .map_err(|error| format!("无法展开窗口：{error}"))?;
    } else {
        let (should_restore_anchor, anchor) = recover_lock(&state.geometry).begin_collapse();
        window
            .set_size(LogicalSize::new(COLLAPSED_SIZE, COLLAPSED_SIZE))
            .map_err(|error| format!("无法收起窗口：{error}"))?;
        if should_restore_anchor {
            if let Some(position) = anchor {
                window
                    .set_position(position)
                    .map_err(|error| format!("无法恢复窗口位置：{error}"))?;
            }
        }
        window
            .app_handle()
            .save_window_state(StateFlags::POSITION)
            .map_err(|error| format!("无法保存窗口位置：{error}"))?;
    }
    Ok(())
}

#[tauri::command]
fn start_dragging(
    expanded: bool,
    window: WebviewWindow,
    state: State<'_, WindowState>,
) -> Result<(), String> {
    if expanded {
        recover_lock(&state.geometry).mark_expanded_drag();
    }
    window
        .start_dragging()
        .map_err(|error| format!("无法拖动窗口：{error}"))
}

#[tauri::command]
fn hide_widget(window: WebviewWindow) -> Result<(), String> {
    window
        .app_handle()
        .save_window_state(StateFlags::POSITION)
        .map_err(|error| format!("无法保存窗口位置：{error}"))?;
    window
        .hide()
        .map_err(|error| format!("无法隐藏窗口：{error}"))
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    let _ = app.save_window_state(StateFlags::POSITION);
    app.exit(0);
}

#[tauri::command]
fn show_widget(window: WebviewWindow) -> Result<(), String> {
    window
        .set_size(LogicalSize::new(COLLAPSED_SIZE, COLLAPSED_SIZE))
        .map_err(|error| format!("无法初始化浮窗尺寸：{error}"))?;
    window
        .show()
        .map_err(|error| format!("无法显示窗口：{error}"))?;
    window
        .set_focus()
        .map_err(|error| format!("无法聚焦窗口：{error}"))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::POSITION)
                .build(),
        )
        .manage(WindowState::default())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("widget") {
                window.set_skip_taskbar(true)?;
                let state_file = app.path().app_config_dir()?.join(".window-state.json");
                if !state_file.exists() {
                    if let Some(monitor) = window.current_monitor()? {
                        let size = window.outer_size()?;
                        let monitor_position = monitor.position();
                        let monitor_size = monitor.size();
                        let x =
                            monitor_position.x + monitor_size.width as i32 - size.width as i32 - 36;
                        let y = monitor_position.y + 116;
                        window.set_position(PhysicalPosition::new(x, y))?;
                    }
                }
            }

            if let Err(error) = setup_tray(app) {
                eprintln!("tray setup failed: {error}");
                if let Some(window) = app.get_webview_window("widget") {
                    let _ = window.set_skip_taskbar(false);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_quota_snapshot,
            get_quota_snapshot_online,
            get_reset_credits_online,
            skin::choose_custom_skin,
            skin::get_custom_skin,
            skin::clear_custom_skin,
            resize_widget,
            start_dragging,
            hide_widget,
            show_widget,
            quit_app
        ])
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Moved(position) => {
                if let Some(state) = window.app_handle().try_state::<WindowState>() {
                    recover_lock(&state.geometry).record_move(*position);
                }
            }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.app_handle().save_window_state(StateFlags::POSITION);
                let _ = window.hide();
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("failed to run the local quota widget");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programmatic_expand_move_does_not_count_as_drag() {
        let anchor = PhysicalPosition::new(1500, 200);
        let clamped = PhysicalPosition::new(1200, 200);
        let mut geometry = WindowGeometry::default();

        geometry.begin_expand(anchor, clamped);
        geometry.record_move(clamped);

        assert!(!geometry.expanded_dragged);
        assert_eq!(geometry.begin_collapse(), (true, Some(anchor)));
    }

    #[test]
    fn native_move_while_expanded_keeps_the_new_position() {
        let anchor = PhysicalPosition::new(1500, 200);
        let clamped = PhysicalPosition::new(1200, 200);
        let dragged = PhysicalPosition::new(900, 360);
        let mut geometry = WindowGeometry::default();

        geometry.begin_expand(anchor, clamped);
        geometry.record_move(clamped);
        geometry.record_move(dragged);

        assert!(geometry.expanded_dragged);
        assert_eq!(geometry.begin_collapse(), (false, Some(anchor)));
    }

    #[test]
    fn collapsed_move_is_not_mistaken_for_expanded_drag() {
        let mut geometry = WindowGeometry::default();

        geometry.record_move(PhysicalPosition::new(640, 480));

        assert!(!geometry.expanded_dragged);
        assert_eq!(geometry.collapsed_anchor, None);
    }
}
