pub mod dialogs;

use std::sync::Arc;
use std::time::Duration;

use cursive::align::HAlign;
use cursive::theme::{BaseColor, Color, Effect, Style};
use cursive::traits::*;
use cursive::utils::markup::StyledString;
use cursive::view::ScrollStrategy;
use cursive::views::{Dialog, LinearLayout, Panel, ScrollView, SelectView, TextArea, TextView};
use cursive::Cursive;
use std::sync::Mutex;
use tokio::sync::mpsc;

use crate::app::{App, StatusLine};
use crate::i18n;
use crate::log::LogBuffer;

#[derive(Debug, Clone)]
pub enum TuiCommand {
    Quit,
    SelectModel(usize),
    DeleteModel(usize),
    DeleteAuxModel(String),
    StartServer,
    StopServer,
    ImportLocal(String),
    SetGithubToken(String),
    SetHostsSource(String),
    ToggleHosts,
    UpdateHosts,
    AddMirror {
        pattern: String,
        replacement: String,
    },
    ToggleMirror(usize),
    ClearMirrors,
    ReloadTranslation,
    SetSilenceMs(i32),
    SetDenoiser {
        model: String,
        voice_sample: String,
    },
}

/// Rough display width: ASCII chars count as 1, everything else as 2.
/// Good enough for our labels (Latin / CJK), not full Unicode-correct.
fn display_width(s: &str) -> usize {
    s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
}

fn pad_to(label: &str, target: usize) -> String {
    let w = display_width(label);
    if w >= target {
        label.to_string()
    } else {
        format!("{label}{}", " ".repeat(target - w))
    }
}

fn help_lines() -> StyledString {
    let s = i18n::t();
    let key_style = Style::from(Color::Light(BaseColor::Cyan)).combine(Effect::Bold);
    let label_style = Style::from(Color::Light(BaseColor::White));
    let dim_style = Style::from(Color::Rgb(128, 128, 128));

    let row_models: [(&str, &str); 4] = [
        ("d", s.help_kw_download),
        ("i", s.help_kw_import),
        ("s", s.help_kw_select),
        ("x", s.help_kw_delete),
    ];
    let row_server: [(&str, &str); 5] = [
        ("r", s.help_kw_run_stop),
        ("o", s.help_kw_settings),
        ("c", s.help_kw_clear_cache),
        ("?", s.help_kw_help),
        ("q", s.help_kw_quit),
    ];

    // Compute per-column max label width across both rows.
    let cols = row_server.len();
    let mut col_widths = vec![0usize; cols];
    for (i, (_, label)) in row_models.iter().enumerate() {
        col_widths[i] = col_widths[i].max(display_width(label));
    }
    for (i, (_, label)) in row_server.iter().enumerate() {
        col_widths[i] = col_widths[i].max(display_width(label));
    }

    // Pad both header prefixes to the same width so the first key starts
    // at the same column on every row.
    let prefix_width = display_width(s.help_kw_models).max(display_width(s.help_kw_server));

    let mut out = StyledString::new();
    let render_row = |out: &mut StyledString, prefix: &str, row: &[(&str, &str)]| {
        out.append_styled(pad_to(prefix, prefix_width), dim_style);
        for (i, (k, label)) in row.iter().enumerate() {
            out.append_styled(format!(" {k} "), key_style);
            out.append_styled(pad_to(label, col_widths[i]), label_style);
            out.append_styled("   ", dim_style);
        }
    };

    render_row(&mut out, s.help_kw_models, &row_models);
    out.append_styled("\n", dim_style);
    render_row(&mut out, s.help_kw_server, &row_server);
    out
}

fn model_list_label(m: &crate::model::ModelInfo) -> String {
    let s = i18n::t();
    let mode = if m.streaming {
        s.list_mode_stream
    } else {
        s.list_mode_offline
    };
    format!("{}  [{mode}]", m.name)
}

pub fn build_ui(
    siv: &mut Cursive,
    app: Arc<Mutex<App>>,
    log_buffer: Arc<LogBuffer>,
    cmd_tx: mpsc::Sender<TuiCommand>,
) {
    siv.set_theme(default_theme());
    disable_terminal_mouse_capture();

    install_main_layout(siv, &app, &cmd_tx);
    bind_events(siv, app.clone(), cmd_tx);

    siv.set_fps(4);

    let app_bg = app;
    let cb_refresh = siv.cb_sink().clone();
    let log_buf_bg = log_buffer;
    std::thread::spawn(move || {
        let mut last_log_len = 0usize;
        let mut last_status_refresh = std::time::Instant::now() - Duration::from_secs(1);
        loop {
            std::thread::sleep(Duration::from_millis(250));

            let current_log_len = log_buf_bg.len();
            if current_log_len != last_log_len {
                last_log_len = current_log_len;
                let lines = log_buf_bg.snapshot();
                let text = lines.join("\n");
                let _ = cb_refresh.send(Box::new(move |siv| {
                    siv.call_on_name("log_view", |v: &mut TextArea| {
                        v.set_content(text);
                    });
                    siv.call_on_name("log_scroll", |v: &mut ScrollView<TextArea>| {
                        v.scroll_to_bottom();
                    });
                }));
            }

            if last_status_refresh.elapsed() >= Duration::from_secs(1) {
                last_status_refresh = std::time::Instant::now();
                let app_status = app_bg.clone();
                let _ = cb_refresh.send(Box::new(move |siv| {
                    let a = app_status.lock().unwrap();
                    let status_lines = a.detailed_status_lines();
                    let models = a.list_models();
                    drop(a);
                    update_status(siv, &status_lines);
                    siv.call_on_name("model_list", |v: &mut SelectView<usize>| {
                        let preserved = v.selected_id();
                        v.clear();
                        for (i, m) in models.iter().enumerate() {
                            v.add_item(model_list_label(m), i);
                        }
                        if let Some(id) = preserved {
                            if id < models.len() {
                                v.set_selection(id);
                            }
                        }
                    });
                }));
            }
        }
    });
}

/// Build (or rebuild) the main fullscreen layout. Idempotent: pops any
/// existing layers first so it can be called when the language changes.
pub(crate) fn install_main_layout(
    siv: &mut Cursive,
    app: &Arc<Mutex<App>>,
    cmd_tx: &mpsc::Sender<TuiCommand>,
) {
    while !siv.screen().is_empty() {
        siv.pop_layer();
    }

    let s = i18n::t();
    let (models, active_model) = {
        let a = app.lock().unwrap();
        (a.list_models(), a.config.active_model.clone())
    };

    let mut model_select = SelectView::new().h_align(HAlign::Left);
    for (i, m) in models.iter().enumerate() {
        model_select.add_item(model_list_label(m), i);
    }
    if let Some(index) = active_model.as_deref().and_then(|active| {
        models
            .iter()
            .position(|model| model.dir == std::path::Path::new(active))
    }) {
        model_select.set_selection(index);
    }
    let tx_select = cmd_tx.clone();
    model_select.set_on_submit(move |_siv, &idx| {
        let _ = tx_select.try_send(TuiCommand::SelectModel(idx));
    });

    let model_view = LinearLayout::vertical()
        .child(TextView::new(s.model_panel_hint).style(Style::from(Color::Rgb(128, 128, 128))))
        .child(
            ScrollView::new(model_select.with_name("model_list"))
                .scroll_x(false)
                .full_height(),
        );

    let model_panel = Panel::new(model_view)
        .title(format!(" {} ", s.title_models))
        .fixed_height(11);

    let log_view = LinearLayout::vertical()
        .child(TextView::new(s.log_panel_hint).style(Style::from(Color::Rgb(128, 128, 128))))
        .child(
            ScrollView::new(TextArea::new().disabled().with_name("log_view"))
                .scroll_strategy(ScrollStrategy::StickToBottom)
                .with_name("log_scroll")
                .full_screen(),
        );

    let log_panel = Panel::new(log_view)
        .title(format!(" {} ", s.title_log))
        .full_screen();

    let status_panel = Panel::new(TextView::new("").with_name("status_panel").full_width())
        .title(format!(" {} ", s.title_status));

    let help_panel = Panel::new(TextView::new(help_lines()).full_width())
        .title(format!(" {} ", s.title_shortcuts));

    let layout = LinearLayout::vertical()
        .child(status_panel)
        .child(model_panel)
        .child(log_panel)
        .weight(1)
        .child(help_panel)
        .full_screen();

    siv.add_fullscreen_layer(layout);

    refresh_status(siv, app);
}

pub fn run_with_terminal_selection(siv: &mut cursive::CursiveRunnable) {
    let mut runner = siv.runner();
    disable_terminal_mouse_capture();
    runner.run();
}

fn disable_terminal_mouse_capture() {
    use std::io::Write;

    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l");
    let _ = stderr.flush();
}

fn bind_events(siv: &mut Cursive, app: Arc<Mutex<App>>, cmd_tx: mpsc::Sender<TuiCommand>) {
    let quit_with_confirm =
        |siv: &mut Cursive, app_ref: &Arc<Mutex<App>>, tx_ref: &mpsc::Sender<TuiCommand>| {
            let running = app_ref.lock().unwrap().server_running;
            if running {
                let s = i18n::t();
                let tx_q = tx_ref.clone();
                siv.add_layer(
                    Dialog::text(s.confirm_quit_body)
                        .title(format!(" {} ", s.confirm_quit_title))
                        .button(s.btn_quit, move |siv: &mut Cursive| {
                            siv.quit();
                            let _ = tx_q.try_send(TuiCommand::Quit);
                        })
                        .button(s.btn_cancel, |siv: &mut Cursive| {
                            siv.pop_layer();
                        }),
                );
            } else {
                siv.quit();
                let _ = tx_ref.try_send(TuiCommand::Quit);
            }
        };

    let tx = cmd_tx.clone();
    let app_q = app.clone();
    siv.add_global_callback('q', move |siv| {
        quit_with_confirm(siv, &app_q, &tx);
    });

    // In terminal raw mode, Ctrl+C is delivered as a key event instead of SIGINT.
    let tx = cmd_tx.clone();
    let app_ctrl_c = app.clone();
    siv.add_global_callback(cursive::event::Event::CtrlChar('c'), move |siv| {
        quit_with_confirm(siv, &app_ctrl_c, &tx);
    });

    let tx = cmd_tx.clone();
    let app_etx = app.clone();
    siv.add_global_callback(cursive::event::Event::Char('\u{3}'), move |siv| {
        quit_with_confirm(siv, &app_etx, &tx);
    });

    siv.add_global_callback(cursive::event::Key::Esc, |siv| {
        if siv.screen().len() > 1 {
            siv.pop_layer();
        }
    });

    let tx = cmd_tx.clone();
    let app_r = app.clone();
    siv.add_global_callback('r', move |_siv| {
        let running = app_r.lock().unwrap().server_running;
        if running {
            let _ = tx.try_send(TuiCommand::StopServer);
        } else {
            let _ = tx.try_send(TuiCommand::StartServer);
        }
    });

    let tx = cmd_tx.clone();
    siv.add_global_callback('x', move |siv| {
        if siv.screen().len() > 1 {
            return;
        }
        let selected = siv.call_on_name("model_list", |v: &mut SelectView<usize>| v.selected_id());
        if let Some(Some(idx)) = selected {
            let tx = tx.clone();
            let s = i18n::t();
            siv.add_layer(
                Dialog::text(
                    s.confirm_delete_body_fmt
                        .replace("{}", &(idx + 1).to_string()),
                )
                .title(format!(" {} ", s.confirm_delete_title))
                .button(s.btn_delete, move |sv| {
                    let _ = tx.try_send(TuiCommand::DeleteModel(idx));
                    sv.pop_layer();
                })
                .button(s.btn_cancel, |sv| {
                    sv.pop_layer();
                }),
            );
        }
    });

    let tx = cmd_tx.clone();
    siv.add_global_callback('s', move |siv| {
        if siv.screen().len() > 1 {
            return;
        }
        let selected = siv.call_on_name("model_list", |v: &mut SelectView<usize>| v.selected_id());
        if let Some(Some(idx)) = selected {
            let _ = tx.try_send(TuiCommand::SelectModel(idx));
        }
    });

    let app_d = app.clone();
    let cb_d = siv.cb_sink().clone();
    siv.add_global_callback('d', move |_siv| {
        dialogs::open_download_browser(app_d.clone(), cb_d.clone());
    });

    let tx_i = cmd_tx.clone();
    siv.add_global_callback('i', move |siv| {
        dialogs::show_import_dialog(siv, tx_i.clone());
    });

    let app_o = app.clone();
    let tx_o = cmd_tx.clone();
    siv.add_global_callback('o', move |siv| {
        show_settings_menu(siv, app_o.clone(), tx_o.clone());
    });

    let app_c = app.clone();
    siv.add_global_callback('c', move |siv| {
        let result = {
            let a = app_c.lock().unwrap();
            crate::download::clear_release_cache(&a.store_dir)
        };
        let s = i18n::t();
        let msg = match result {
            Ok(()) => s.cache_cleared.to_string(),
            Err(e) => s.cache_failed_fmt.replace("{}", &e.to_string()),
        };
        siv.add_layer(
            Dialog::text(msg)
                .title(format!(" {} ", s.cache_title))
                .button(s.btn_ok, |sv| {
                    sv.pop_layer();
                }),
        );
    });

    siv.add_global_callback('?', dialogs::show_help_dialog);
    siv.add_global_callback(cursive::event::Key::F1, dialogs::show_help_dialog);

    // j/k act as Down/Up when focus isn't on a text input. Global callbacks
    // only fire on events ignored by the focused view, so TextArea/EditView
    // still receive the keystrokes verbatim while typing.
    siv.add_global_callback('j', |siv| {
        siv.on_event(cursive::event::Event::Key(cursive::event::Key::Down));
    });
    siv.add_global_callback('k', |siv| {
        siv.on_event(cursive::event::Event::Key(cursive::event::Key::Up));
    });
}

fn show_denoiser_dialog(siv: &mut Cursive, app: Arc<Mutex<App>>, cmd_tx: mpsc::Sender<TuiCommand>) {
    let (model, voice_sample, models_dir, store_dir) = {
        let a = app.lock().unwrap();
        (
            a.config.denoiser_model.clone().unwrap_or_default(),
            a.config.voxlock_voice_sample.clone().unwrap_or_default(),
            crate::model::models_dir(&a.store_dir),
            a.store_dir.clone(),
        )
    };
    let s = i18n::t();
    let mut model_select = SelectView::<String>::new();
    model_select.add_item(s.denoiser_disabled, String::new());
    let mut candidates = installed_denoisers(&models_dir);
    if !model.is_empty() && !candidates.iter().any(|p| p == &model) {
        candidates.push(model.clone());
    }
    candidates.sort();
    for candidate in candidates {
        let label = denoiser_label(&candidate);
        model_select.add_item(label, candidate);
    }
    let selected = model_select
        .iter()
        .position(|(_, value)| value == &model)
        .unwrap_or(0);
    model_select.set_selection(selected);
    let model_select = model_select.with_name("denoiser_model");
    let voice_path = TextView::new(voice_sample).with_name("voxlock_voice_sample_path");
    let tx = cmd_tx.clone();
    siv.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new(s.denoiser_model_label))
                .child(model_select)
                .child(TextView::new(s.denoiser_voice_label))
                .child(TextView::new(s.denoiser_voice_hint))
                .child(voice_path),
        )
        .title(format!(" {} ", s.denoiser_title))
        .button(s.denoiser_browse, move |siv| {
            dialogs::show_voice_sample_dialog(siv, store_dir.clone());
        })
        .button(s.denoiser_save_reload, move |siv| {
            let model = siv
                .call_on_name("denoiser_model", |v: &mut SelectView<String>| {
                    v.selection().map(|s| s.to_string()).unwrap_or_default()
                })
                .unwrap_or_default();
            let voice_sample = siv
                .call_on_name("voxlock_voice_sample_path", |v: &mut TextView| {
                    v.get_content().source().to_string()
                })
                .unwrap_or_default();
            let _ = tx.try_send(TuiCommand::SetDenoiser {
                model,
                voice_sample,
            });
            siv.pop_layer();
        })
        .button(s.denoiser_disable, {
            let tx = cmd_tx.clone();
            move |siv| {
                let _ = tx.try_send(TuiCommand::SetDenoiser {
                    model: String::new(),
                    voice_sample: String::new(),
                });
                siv.pop_layer();
            }
        })
        .button(s.denoiser_cancel, |siv| {
            siv.pop_layer();
        }),
    );
}

fn show_settings_menu(siv: &mut Cursive, app: Arc<Mutex<App>>, cmd_tx: mpsc::Sender<TuiCommand>) {
    let s = i18n::t();
    let mut menu = SelectView::<String>::new();
    menu.add_item(s.settings_network, "network".to_string());
    menu.add_item(s.settings_translation, "translation".to_string());
    menu.add_item(s.settings_denoiser, "denoiser".to_string());
    menu.add_item(s.settings_manage_models, "manage_models".to_string());
    menu.add_item(s.settings_language, "language".to_string());
    let app_menu = app.clone();
    let tx_menu = cmd_tx.clone();
    menu.set_on_submit(move |siv, action: &String| {
        let action = action.clone();
        siv.pop_layer();
        match action.as_str() {
            "network" => dialogs::show_network_dialog(siv, app_menu.clone(), tx_menu.clone()),
            "translation" => {
                dialogs::show_translation_settings(siv, app_menu.clone(), tx_menu.clone())
            }
            "denoiser" => show_denoiser_dialog(siv, app_menu.clone(), tx_menu.clone()),
            "manage_models" => {
                dialogs::show_aux_models_dialog(siv, app_menu.clone(), tx_menu.clone())
            }
            "language" => dialogs::show_language_dialog(siv, app_menu.clone(), tx_menu.clone()),
            _ => {}
        }
    });
    siv.add_layer(
        Dialog::around(menu.scrollable().fixed_height(8))
            .title(format!(" {} ", s.settings_title))
            .button(s.btn_cancel, |siv| {
                siv.pop_layer();
            }),
    );
}

fn denoiser_label(path: &str) -> String {
    let name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    if name.to_lowercase().contains("voxlock") {
        "VoxLock".to_string()
    } else if name.to_lowercase().contains("dpdfnet") {
        format!("DPDFNet ({name})")
    } else {
        format!("GTCRN ({name})")
    }
}

fn installed_denoisers(models_dir: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    fn walk(path: &std::path::Path, files: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("onnx") || e.eq_ignore_ascii_case("ort"))
            {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if name.contains("voxlock") && name.contains("enroll") {
                    continue;
                }
                if name.contains("gtcrn")
                    || name.contains("dpdfnet")
                    || (name.contains("voxlock") && name.contains("chunk"))
                {
                    files.push(path.to_string_lossy().into_owned());
                }
            }
        }
    }
    walk(models_dir, &mut files);
    files
}

pub(crate) fn update_status(siv: &mut Cursive, lines: &[StatusLine]) {
    let dim = Style::from(Color::Rgb(128, 128, 128));
    let key = Style::from(Color::Light(BaseColor::Cyan)).combine(Effect::Bold);
    let warn = Style::from(Color::Light(BaseColor::Red)).combine(Effect::Bold);
    let mut content = StyledString::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            content.append_plain("\n");
        }
        if line.text.is_empty() {
            continue;
        }
        if line.warn {
            content.append_styled(&line.text, warn);
        } else if let Some((label, value)) = line.text.split_once(':') {
            content.append_styled(label, key);
            content.append_styled(":", dim);
            content.append_plain(value);
        } else {
            content.append_plain(line.text.as_str());
        }
    }
    siv.call_on_name("status_panel", |v: &mut TextView| {
        v.set_content(content);
    });
}

fn refresh_status(siv: &mut Cursive, app: &Arc<Mutex<App>>) {
    let lines = app.lock().unwrap().detailed_status_lines();
    update_status(siv, &lines);
}

fn default_theme() -> cursive::theme::Theme {
    use cursive::theme::{BorderStyle, Palette, PaletteColor, Theme};
    let mut palette = Palette::default();
    palette[PaletteColor::Background] = Color::TerminalDefault;
    palette[PaletteColor::View] = Color::TerminalDefault;
    palette[PaletteColor::Primary] = Color::Light(BaseColor::White);
    palette[PaletteColor::Secondary] = Color::Rgb(180, 180, 180);
    palette[PaletteColor::TitlePrimary] = Color::Light(BaseColor::Cyan);
    palette[PaletteColor::Highlight] = Color::Rgb(0x20, 0x55, 0xb0);
    palette[PaletteColor::HighlightInactive] = Color::Rgb(0x55, 0x55, 0x55);
    palette[PaletteColor::HighlightText] = Color::Light(BaseColor::White);
    Theme {
        shadow: false,
        borders: BorderStyle::Simple,
        palette,
    }
}
