use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};

fn focus_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        // Back to a normal app while a window is on screen, or an Accessory app
        // cannot properly take focus and the window opens behind whatever the
        // user was in. Dropped back to Accessory when the window is closed.
        #[cfg(target_os = "macos")]
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Put the most recent transcript back on the clipboard. The pipeline restores
/// the user's clipboard after pasting, so this is the only way to get a
/// transcript back without opening the history view.
fn copy_last_transcript(app: &tauri::AppHandle) {
    let state = app.state::<crate::AppState>();
    let db_guard = match state.db.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let Some(db) = db_guard.as_ref() else { return };

    match db.recent(1) {
        Ok(rows) => match rows.first() {
            Some(t) => {
                let copied = arboard::Clipboard::new()
                    .and_then(|mut c| c.set_text(t.text.clone()));
                match copied {
                    Ok(()) => log::info!("Last transcript copied to clipboard"),
                    Err(e) => log::warn!("Failed to copy last transcript: {}", e),
                }
            }
            None => log::info!("No transcript to copy yet"),
        },
        Err(e) => log::warn!("Failed to read last transcript: {}", e),
    }
}

pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItemBuilder::with_id("show", "Show Inkwell").build(app)?;
    let copy_item = MenuItemBuilder::with_id("copy-last", "Copy Last Transcript").build(app)?;
    let settings_item = MenuItemBuilder::with_id("settings", "Settings...").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let tray_menu = MenuBuilder::new(app)
        .item(&show_item)
        .item(&copy_item)
        .separator()
        .item(&settings_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let builder = TrayIconBuilder::new()
        .icon(app.default_window_icon().cloned().unwrap())
        .tooltip("Inkwell")
        .menu(&tray_menu);

    // macOS renders menu bar icons from the alpha channel only when they are
    // flagged as templates; without this the icon is a colour blob that goes
    // invisible against a dark menu bar.
    #[cfg(target_os = "macos")]
    let builder = builder.icon_as_template(true);

    let _tray = builder
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => focus_main(app),
            "copy-last" => copy_last_transcript(app),
            "settings" => {
                focus_main(app);
                let _ = app.emit("open-settings", ());
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                ..
            } = event
            {
                focus_main(tray.app_handle());
            }
        })
        .build(app)?;

    log::info!("System tray initialized");
    Ok(())
}
