mod commands;

use ov_core::dreaming::db::DesktopDb;
use ov_core::dreaming::engine::run_dreaming;
use ov_core::ov_client::OvClient;
use ov_core::platform::PlatformPaths;
use ov_core::policy::model::DreamingTrigger;
use ov_core::policy::store::{normalize_cron, PolicyStore};
use std::str::FromStr;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            let _ = PolicyStore::load_or_init();
            if let Ok(db) = DesktopDb::open_default() {
                let _ = db.recover_stale_claims();
            }
            if let Some(paths) = PlatformPaths::detect() {
                let _ = paths.ensure_bridge_installed(None);
            }

            let open_item =
                MenuItem::with_id(app, "open", "打开 OpenViking 控制台", true, None::<&str>)?;
            let privacy_item = MenuItem::with_id(
                app,
                "toggle_privacy",
                "切换隐私模式（暂停捕获）",
                true,
                None::<&str>,
            )?;
            let dream_item = MenuItem::with_id(
                app,
                "run_dreaming",
                "立即执行 Dreaming 整理",
                true,
                None::<&str>,
            )?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_item, &privacy_item, &dream_item, &quit_item])?;

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("OpenViking Desktop — Agent Memory Control Plane")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                    "toggle_privacy" => {
                        if let Ok(mut policy) = PolicyStore::load_or_init() {
                            policy.global.capture_enabled = !policy.global.capture_enabled;
                            let _ = PolicyStore::save(&policy);
                            let _ =
                                app.emit("policy://privacy-changed", policy.global.capture_enabled);
                        }
                    }
                    "run_dreaming" => {
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let (Ok(client), Ok(db), Ok(policy)) = (
                                OvClient::from_local_config(),
                                DesktopDb::open_default(),
                                PolicyStore::load_or_init(),
                            ) {
                                if let Ok((run, staged)) =
                                    run_dreaming(&client, &db, &policy, "tray_manual").await
                                {
                                    let _ =
                                        handle.emit("dreaming://completed", (run, staged.len()));
                                }
                            }
                        });
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                });

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            #[cfg(target_os = "macos")]
            {
                tray_builder = tray_builder.icon_as_template(true);
            }
            let _tray = tray_builder.build(app)?;

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut last_poll = chrono::Utc::now();
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    let now = chrono::Utc::now();
                    let since = std::mem::replace(&mut last_poll, now);
                    let Ok(policy) = PolicyStore::load_or_init() else {
                        continue;
                    };
                    let dreaming = &policy.lifecycle.dreaming;
                    if !dreaming.enabled || dreaming.trigger == DreamingTrigger::Manual {
                        continue;
                    }
                    let (Ok(client), Ok(db)) =
                        (OvClient::from_local_config(), DesktopDb::open_default())
                    else {
                        continue;
                    };
                    let profile_key = client.profile_key();
                    let source = match dreaming.trigger {
                        DreamingTrigger::Schedule => {
                            let expr = normalize_cron(&dreaming.cron);
                            let Ok(schedule) = cron::Schedule::from_str(&expr) else {
                                continue;
                            };
                            let Some(slot) =
                                schedule.after(&since).take_while(|at| *at <= now).last()
                            else {
                                continue;
                            };
                            if !matches!(
                                db.claim_schedule_slot(&profile_key, slot.timestamp()),
                                Ok(true)
                            ) {
                                continue;
                            }
                            "schedule"
                        }
                        DreamingTrigger::SessionThreshold => {
                            let Ok(runs) = db.list_runs_for_profile(&profile_key, 1) else {
                                continue;
                            };
                            let last_ts = runs.first().map_or(0, |run| run.started_at);
                            let Ok(new_sessions) = db.count_sessions_since(&profile_key, last_ts) else {
                                continue;
                            };
                            if new_sessions < i64::from(dreaming.session_threshold) {
                                continue;
                            }
                            "session_threshold"
                        }
                        DreamingTrigger::Manual => continue,
                    };

                    if let Ok((run, staged)) = run_dreaming(&client, &db, &policy, source).await {
                        let _ = app_handle.emit("dreaming://completed", (run, staged.len()));
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_server_status,
            commands::test_connection,
            commands::get_profiles,
            commands::save_profiles,
            commands::detect_agents,
            commands::get_capture_health,
            commands::sync_capture_now,
            commands::detect_wsl,
            commands::connect_wsl_agent,
            commands::connect_agent,
            commands::disconnect_agent,
            commands::import_legacy_memory,
            commands::get_policy,
            commands::save_policy,
            commands::toggle_privacy_mode,
            commands::fs_ls,
            commands::fs_mkdir,
            commands::fs_delete,
            commands::search_find,
            commands::read_uri_layers,
            commands::write_uri_content,
            commands::list_recall_events,
            commands::list_recall_hits,
            commands::trigger_dreaming,
            commands::list_dreaming_runs,
            commands::list_pending_changes,
            commands::update_change_decision,
            commands::apply_accepted_changes,
            commands::rollback_dreaming_run,
            commands::pin_memory_uri,
        ])
        .run(tauri::generate_context!())
        .expect("error while running OpenViking Desktop");
}
