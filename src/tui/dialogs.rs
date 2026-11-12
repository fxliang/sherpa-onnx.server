use std::path::{Path, PathBuf};
use std::sync::Arc;

use cursive::theme::{BaseColor, Color, Effect, Style};
use cursive::traits::*;
use cursive::utils::markup::StyledString;
use cursive::views::{
    Checkbox, Dialog, DummyView, EditView, LinearLayout, Panel, ScrollView, SelectView, TextView,
};
use cursive::CbSink;
use cursive::Cursive;
use std::sync::Mutex;
use tokio::sync::mpsc;
use tracing::info;

use crate::app::App;
use crate::download;
use crate::i18n;
use crate::model::{AuxKind, AuxModel};

use super::TuiCommand;

pub fn show_help_dialog(siv: &mut Cursive) {
    let s = i18n::t();
    let mut body = StyledString::new();
    let header = Style::from(Color::Light(BaseColor::Cyan)).combine(Effect::Bold);
    let key = Style::from(Color::Light(BaseColor::Yellow)).combine(Effect::Bold);
    let plain = Style::from(Color::Light(BaseColor::White));

    body.append_styled(s.help_section_getting_started, header);
    body.append_styled(s.help_getting_started_1_a, plain);
    body.append_styled("d", key);
    body.append_styled(s.help_getting_started_1_b, plain);
    body.append_styled("i", key);
    body.append_styled(s.help_getting_started_1_c, plain);
    body.append_styled(s.help_getting_started_2_a, plain);
    body.append_styled("s", key);
    body.append_styled(s.help_getting_started_2_b, plain);
    body.append_styled(s.help_getting_started_3_a, plain);
    body.append_styled("r", key);
    body.append_styled(s.help_getting_started_3_b, plain);
    body.append_plain("\n");

    body.append_styled(s.help_section_keys, header);
    for (k, desc) in [
        ("d", s.help_key_d),
        ("i", s.help_key_i),
        ("s", s.help_key_s),
        ("Enter", s.help_key_enter),
        ("x", s.help_key_x),
        ("r", s.help_key_r),
        ("o", s.help_key_o),
        ("c", s.help_key_c),
        ("?", s.help_key_help),
        ("q", s.help_key_q),
    ] {
        body.append_plain("  ");
        body.append_styled(format!("{k:<6}"), key);
        body.append_styled(desc, plain);
        body.append_plain("\n");
    }
    body.append_plain("\n");

    body.append_styled(s.help_section_files, header);
    body.append_styled(s.help_files_default, plain);
    body.append_styled(s.help_files_models, plain);
    body.append_styled(s.help_files_punct, plain);
    body.append_styled(s.help_files_config, plain);
    body.append_styled(s.help_files_cache, plain);
    body.append_plain("\n");

    body.append_styled(s.help_section_network, header);
    body.append_styled(s.help_network_press_a, plain);
    body.append_styled("n", key);
    body.append_styled(s.help_network_press_b, plain);
    body.append_styled(s.help_network_token, plain);
    body.append_styled(s.help_network_mirror, plain);
    body.append_styled(s.help_network_mirror_example, plain);
    body.append_styled(s.help_network_hosts_a, plain);
    body.append_styled(s.help_network_hosts_b, plain);
    body.append_plain("\n");

    body.append_styled(s.help_section_models, header);
    body.append_styled(s.help_models_list_1, plain);
    body.append_styled(s.help_models_list_2, plain);
    body.append_styled(s.help_models_aux_1, plain);
    body.append_styled(s.help_models_aux_2, plain);

    siv.add_layer(
        Dialog::around(ScrollView::new(TextView::new(body)).fixed_height(24))
            .title(format!(" {} ", s.help_title))
            .button(s.btn_close, |sv| {
                sv.pop_layer();
            })
            .min_width(78),
    );
}

pub fn open_download_browser(app: Arc<Mutex<App>>, cb: CbSink) {
    fetch_then_show(app, cb, "recommended".to_string(), false);
}

fn fetch_then_show(app: Arc<Mutex<App>>, cb: CbSink, category: String, force_refresh: bool) {
    let _ = cb.send(Box::new(|siv| {
        let s = i18n::t();
        siv.add_layer(
            Dialog::text(s.dl_loading)
                .title(format!(" {} ", s.dl_title))
                .with_name("download_loading"),
        );
    }));

    std::thread::spawn(move || {
        info!(
            "Fetching release assets (category={}, force={})",
            category, force_refresh
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (assets, err) = rt.block_on(async {
            let (store_dir, config, http, force_refresh) = {
                let a = app.lock().unwrap();
                (
                    a.store_dir.clone(),
                    a.config.clone(),
                    a.http.clone(),
                    force_refresh,
                )
            };
            match download::fetch_release_assets(&store_dir, &config, &http, force_refresh).await {
                Ok(a) => (a, None),
                Err(e) => (Vec::new(), Some(format!("{e}"))),
            }
        });

        let app2 = app.clone();
        let cb2 = cb.clone();
        let _ = cb.send(Box::new(move |siv| {
            if siv.find_name::<Dialog>("download_loading").is_some() {
                siv.pop_layer();
            }
            let s = i18n::t();
            if let Some(err) = err {
                siv.add_layer(
                    Dialog::text(s.dl_fetch_failed_body_fmt.replace("{}", &err))
                        .title(format!(" {} ", s.dl_fetch_failed_title))
                        .button(s.btn_ok, |sv| {
                            sv.pop_layer();
                        }),
                );
                return;
            }
            if assets.is_empty() {
                siv.add_layer(Dialog::text(s.dl_no_assets).button(s.btn_ok, |sv| {
                    sv.pop_layer();
                }));
                return;
            }
            let models_dir = app2.lock().unwrap().store_dir.join("models");
            let assets = assets
                .into_iter()
                .filter(|asset| !asset_already_installed(&models_dir, &asset.name))
                .collect();
            build_download_dialog(siv, app2, cb2, assets, category);
        }));
    });
}

fn build_download_dialog(
    siv: &mut Cursive,
    app: Arc<Mutex<App>>,
    cb: CbSink,
    assets: Vec<download::Asset>,
    initial_category: String,
) {
    let s = i18n::t();
    let assets_arc: Arc<Vec<download::Asset>> = Arc::new(assets);

    let mut category_select = SelectView::<String>::new();
    for (key, desc) in download::CATEGORIES {
        let label = format!("{key:<12}  {desc}");
        category_select.add_item(label, key.to_string());
    }
    if let Some(pos) = download::CATEGORIES
        .iter()
        .position(|(k, _)| *k == initial_category)
    {
        category_select.set_selection(pos);
    }

    {
        let assets_for_cat = assets_arc.clone();
        category_select.set_on_select(move |siv, value| {
            let filter = read_filter(siv);
            populate_dl_list(siv, &assets_for_cat, value, &filter);
        });
    }

    let filter_edit = {
        let assets_for_filter = assets_arc.clone();
        EditView::new()
            .on_submit(move |siv, _content| {
                let cat = read_category(siv);
                let filter = read_filter(siv);
                populate_dl_list(siv, &assets_for_filter, &cat, &filter);
            })
            .with_name("dl_filter")
            .fixed_width(40)
    };

    let mut asset_select =
        SelectView::<download::Asset>::new().h_align(cursive::align::HAlign::Left);
    {
        let app_submit = app.clone();
        let cb_submit = cb.clone();
        asset_select.set_on_submit(move |siv, asset: &download::Asset| {
            if asset.url.is_empty() {
                return;
            }
            let asset = asset.clone();
            let app = app_submit.clone();
            let cb = cb_submit.clone();
            let s = i18n::t();
            let body = s
                .dl_confirm_body_fmt
                .replace("{name}", &asset.name)
                .replace("{size}", &download::format_bytes(asset.size))
                .replace("{kind}", &asset.kind)
                .replace("{release}", &asset.release);
            siv.add_layer(
                Dialog::text(body)
                    .title(format!(" {} ", s.dl_confirm_title))
                    .button(s.btn_download, move |siv| {
                        siv.pop_layer(); // confirm
                        siv.pop_layer(); // asset list

                        let s = i18n::t();
                        let progress_title = format!(" {}{} ", s.dl_progress_title_prefix, asset.name);
                        siv.add_layer(
                            Dialog::around(
                                LinearLayout::vertical()
                                    .child(
                                        TextView::new(s.dl_progress_connecting)
                                            .with_name("dl_progress_text")
                                            .min_width(60),
                                    )
                                    .child(DummyView.fixed_height(1))
                                    .child(
                                        TextView::new("")
                                            .with_name("dl_progress_bar")
                                            .min_width(60),
                                    ),
                            )
                            .title(progress_title)
                            .with_name("dl_progress_dialog"),
                        );

                        let app = app.clone();
                        let cb = cb.clone();
                        let asset = asset.clone();
                        std::thread::spawn(move || {
                            let cb_progress = cb.clone();
                            let last_emit = std::sync::Arc::new(std::sync::Mutex::new(
                                std::time::Instant::now() - std::time::Duration::from_secs(1),
                            ));
                            let start = std::time::Instant::now();
                            let on_progress = move |done: u64, total: u64| {
                                let mut last = match last_emit.lock() {
                                    Ok(g) => g,
                                    Err(_) => return,
                                };
                                let now = std::time::Instant::now();
                                if now.duration_since(*last)
                                    < std::time::Duration::from_millis(100)
                                    && (total == 0 || done < total)
                                {
                                    return;
                                }
                                *last = now;
                                let elapsed = start.elapsed().as_secs_f64().max(1e-3);
                                let speed = done as f64 / elapsed;
                                let (line, bar) = if total > 0 {
                                    let pct = (done * 100 / total).min(100);
                                    let bar_width = 40usize;
                                    let filled = (pct as usize * bar_width) / 100;
                                    let bar = format!(
                                        "[{}{}] {pct:3}%",
                                        "=".repeat(filled),
                                        " ".repeat(bar_width - filled),
                                    );
                                    let eta = if speed > 0.0 && total > done {
                                        let secs = ((total - done) as f64 / speed) as u64;
                                        format!(", ETA {}s", secs)
                                    } else {
                                        String::new()
                                    };
                                    (
                                        format!(
                                            "{} / {}  ({}/s{})",
                                            crate::download::format_bytes(done),
                                            crate::download::format_bytes(total),
                                            crate::download::format_bytes(speed as u64),
                                            eta,
                                        ),
                                        bar,
                                    )
                                } else {
                                    (
                                        format!(
                                            "{}  ({}/s)",
                                            crate::download::format_bytes(done),
                                            crate::download::format_bytes(speed as u64),
                                        ),
                                        String::new(),
                                    )
                                };
                                let _ = cb_progress.send(Box::new(move |siv| {
                                    siv.call_on_name("dl_progress_text", |v: &mut TextView| {
                                        v.set_content(line);
                                    });
                                    siv.call_on_name("dl_progress_bar", |v: &mut TextView| {
                                        v.set_content(bar);
                                    });
                                }));
                            };
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .unwrap();
                            rt.block_on(async {
                                let (http, store_cfg, _store_dir, tmp_dir) = {
                                    let a = app.lock().unwrap();
                                    (
                                        a.http.clone(),
                                        a.config.clone(),
                                        a.store_dir.clone(),
                                        a.store_dir.join("models").join(".tmp"),
                                    )
                                };

                                let _ = std::fs::create_dir_all(&tmp_dir);
                                let archive_path = tmp_dir.join(&asset.name);

                                let result = http
                                    .download_file(
                                        &asset.url,
                                        &archive_path,
                                        &store_cfg,
                                        on_progress,
                                    )
                                    .await;

                                let import_result = match result {
                                    Ok(()) => {
                                        let mut a = app.lock().unwrap();
                                        let r = crate::model::import::import_path(
                                            &archive_path,
                                            &crate::model::models_dir(&a.store_dir),
                                        );
                                        match r {
                                            Ok(imported) => {
                                                std::fs::remove_file(&archive_path).ok();
                                                let should_reload_runtime;
                                                if asset.kind == "punctuation" {
                                                    a.config.punctuation_model = Some(
                                                        imported.to_string_lossy().into_owned(),
                                                    );
                                                    should_reload_runtime = a.server_running;
                                                } else if asset.kind == "vad" {
                                                    a.config.vad_model = Some(
                                                        imported.to_string_lossy().into_owned(),
                                                    );
                                                    should_reload_runtime = a.server_running;
                                                } else if asset.kind == "translation" {
                                                    // Find the .gguf file inside the imported directory
                                                    let gguf_path = std::fs::read_dir(&imported)
                                                        .ok()
                                                        .and_then(|mut entries| {
                                                            entries.find_map(|e| {
                                                                let e = e.ok()?;
                                                                let is_gguf = e
                                                                    .path()
                                                                    .extension()
                                                                    .and_then(|ext| ext.to_str())
                                                                    .map(|ext| {
                                                                        ext.eq_ignore_ascii_case("gguf")
                                                                    })
                                                                    .unwrap_or(false);
                                                                if is_gguf {
                                                                    Some(e.path())
                                                                } else {
                                                                    None
                                                                }
                                                            })
                                                        })
                                                        .unwrap_or_else(|| imported.clone());
                                                    a.config.translation_model = Some(
                                                        gguf_path.to_string_lossy().into_owned(),
                                                    );
                                                    a.config.translation_enabled = true;
                                                    should_reload_runtime = false;
                                                } else if asset.kind == "speaker" {
                                                    let imported_str = imported.to_string_lossy().into_owned();
                                                    let model_path = crate::engine::resolve_model_file(&imported_str)
                                                        .unwrap_or(imported_str);
                                                    a.config.speaker_model = Some(model_path);
                                                    should_reload_runtime = a.server_running;
                                                } else if asset.kind == "denoiser" {
                                                    let imported_str = imported.to_string_lossy().into_owned();
                                                    let model_path = crate::engine::resolve_model_file(&imported_str)
                                                        .unwrap_or(imported_str);
                                                    a.config.denoiser_model = Some(model_path);
                                                    should_reload_runtime = a.server_running;
                                                } else {
                                                    a.config.active_model = Some(
                                                        imported.to_string_lossy().into_owned(),
                                                    );
                                                    should_reload_runtime = a.server_running;
                                                }
                                                let _ = a.config.save(&a.store_dir);
                                                if should_reload_runtime {
                                                    if let Err(e) = a.reload_active_runtime() {
                                                        tracing::error!(
                                                            "Runtime reload failed after import: {e}"
                                                        );
                                                    }
                                                }
                                                tracing::info!(
                                                    "Imported to {}",
                                                    crate::path_display::for_log_path(&imported)
                                                );
                                                Ok(())
                                            }
                                            Err(e) => Err(e),
                                        }
                                    }
                                    Err(e) => Err(e),
                                };

                                match import_result {
                                    Ok(()) => {
                                        let app_refresh = app.clone();
                                        let _ = cb.send(Box::new(move |siv| {
                                            if siv
                                                .find_name::<Dialog>("dl_progress_dialog")
                                                .is_some()
                                            {
                                                siv.pop_layer();
                                            }
                                            {
                                                let a = app_refresh.lock().unwrap();
                                                let models = a.list_models();
                                                let status_lines = a.detailed_status_lines();
                                                drop(a);
                                                super::update_status(siv, &status_lines);
                                                siv.call_on_name(
                                                    "model_list",
                                                    |v: &mut SelectView<usize>| {
                                                        let preserved = v.selected_id();
                                                        v.clear();
                                                        for (i, m) in models.iter().enumerate() {
                                                            v.add_item(
                                                                super::model_list_label(m),
                                                                i,
                                                            );
                                                        }
                                                        if let Some(id) = preserved {
                                                            if id < models.len() {
                                                                v.set_selection(id);
                                                            }
                                                        }
                                                    },
                                                );
                                            }
                                            let s = i18n::t();
                                            siv.add_layer(
                                                Dialog::text(s.dl_done_body)
                                                    .title(format!(" {} ", s.dl_done_title))
                                                    .button(s.btn_ok, |sv| {
                                                        sv.pop_layer();
                                                    }),
                                            );
                                        }));
                                    }
                                    Err(e) => {
                                        let _ = cb.send(Box::new(move |siv| {
                                            if siv
                                                .find_name::<Dialog>("dl_progress_dialog")
                                                .is_some()
                                            {
                                                siv.pop_layer();
                                            }
                                            let s = i18n::t();
                                            let msg =
                                                s.dl_error_body_fmt.replace("{}", &e.to_string());
                                            siv.add_layer(
                                                Dialog::text(msg)
                                                    .title(format!(" {} ", s.dl_error_title))
                                                    .button(s.btn_ok, |sv| {
                                                        sv.pop_layer();
                                                    }),
                                            );
                                        }));
                                    }
                                }
                            });
                        });
                    })
                    .button(s.btn_cancel, |sv| {
                        sv.pop_layer();
                    }),
            );
        });
    }

    let asset_select_named = asset_select.with_name("dl_list");

    let screen = siv.screen_size();
    let dialog_width = if screen.x > 20 {
        screen.x.saturating_sub(4).clamp(60, 100)
    } else {
        84
    };
    let asset_height = if screen.y > 20 {
        screen.y.saturating_sub(18).clamp(6, 16)
    } else {
        12
    };

    let category_panel = Panel::new(
        ScrollView::new(category_select.with_name("dl_category"))
            .scroll_x(false)
            .fixed_height(8),
    )
    .title(format!(" {} ", s.dl_category_title));

    let filter_row = LinearLayout::horizontal()
        .child(TextView::new(s.dl_filter_label).style(Style::from(Color::Light(BaseColor::Yellow))))
        .child(filter_edit);

    let asset_list_panel = Panel::new(
        ScrollView::new(asset_select_named)
            .scroll_x(true)
            .fixed_height(asset_height),
    )
    .title(format!(" {} ", s.dl_assets_title));

    let hint = TextView::new(s.dl_filter_hint).style(Style::from(Color::Rgb(128, 128, 128)));

    let body = LinearLayout::vertical()
        .child(category_panel)
        .child(DummyView.fixed_height(1))
        .child(filter_row)
        .child(DummyView.fixed_height(1))
        .child(asset_list_panel)
        .child(DummyView.fixed_height(1))
        .child(hint);

    let app_refresh = app.clone();
    let cb_refresh = cb.clone();
    let dialog = Dialog::around(body)
        .title(format!(" {} ", s.dl_title))
        .button(s.btn_refresh, move |siv| {
            let cat = read_category(siv);
            siv.pop_layer();
            fetch_then_show(app_refresh.clone(), cb_refresh.clone(), cat, true);
        })
        .button(s.btn_close, |sv| {
            sv.pop_layer();
        })
        .fixed_width(dialog_width);

    siv.add_layer(dialog);

    let assets_initial = assets_arc.clone();
    let cat_initial = initial_category;
    populate_dl_list(siv, &assets_initial, &cat_initial, "");
}

fn read_category(siv: &mut Cursive) -> String {
    siv.call_on_name("dl_category", |v: &mut SelectView<String>| {
        v.selection().map(|s| s.to_string())
    })
    .flatten()
    .unwrap_or_else(|| "recommended".to_string())
}

fn read_filter(siv: &mut Cursive) -> String {
    siv.call_on_name("dl_filter", |v: &mut EditView| v.get_content())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn populate_dl_list(siv: &mut Cursive, assets: &[download::Asset], category: &str, filter: &str) {
    let filtered: Vec<download::Asset> = download::filter_assets_by_category(assets, category)
        .into_iter()
        .filter(|a| asset_name_matches_filter(&a.name, filter))
        .cloned()
        .collect();
    siv.call_on_name("dl_list", |v: &mut SelectView<download::Asset>| {
        v.clear();
        if filtered.is_empty() {
            v.add_item(
                i18n::t().dl_no_matches,
                download::Asset {
                    name: String::new(),
                    url: String::new(),
                    size: 0,
                    release: String::new(),
                    kind: String::new(),
                },
            );
            return;
        }
        for asset in &filtered {
            let label = format!(
                "[{}] {}  ({}, {})",
                asset.kind,
                asset.name,
                download::format_bytes(asset.size),
                asset.release,
            );
            v.add_item(label, asset.clone());
        }
    });
}

fn asset_name_matches_filter(name: &str, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }
    let has_regex_meta = filter.chars().any(|c| {
        matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '|' | '^' | '$'
        )
    });

    if has_regex_meta {
        if let Ok(re) = regex::RegexBuilder::new(filter)
            .case_insensitive(true)
            .build()
        {
            return re.is_match(name);
        }
    }

    name.to_lowercase().contains(&filter.to_lowercase())
}

fn asset_already_installed(models_dir: &std::path::Path, asset_name: &str) -> bool {
    let target = asset_name
        .to_lowercase()
        .trim_end_matches(".tar.bz2")
        .trim_end_matches(".tar.gz")
        .trim_end_matches(".tgz")
        .trim_end_matches(".zip")
        .trim_end_matches(".onnx")
        .trim_end_matches(".ort")
        .to_string();
    fn walk(path: &std::path::Path, target: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(path) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if name == target || name.starts_with(&(target.to_string() + "-")) {
                return true;
            }
            if path.is_dir() && walk(&path, target) {
                return true;
            }
        }
        false
    }
    walk(models_dir, &target)
}

fn start_model_import(siv: &mut Cursive, path: &str, tx: &mpsc::Sender<TuiCommand>) {
    let path = path.trim().to_string();
    if path.is_empty() {
        let s = i18n::t();
        siv.add_layer(Dialog::text(s.import_empty_path).button(s.btn_ok, |sv| {
            sv.pop_layer();
        }));
        return;
    }
    let _ = tx.try_send(TuiCommand::ImportLocal(path));
    siv.pop_layer();
}

pub fn show_import_dialog(siv: &mut Cursive, cmd_tx: mpsc::Sender<TuiCommand>) {
    let s = i18n::t();
    let tx = cmd_tx;
    let start_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));

    let mut path_edit = EditView::new().content(start_dir.to_string_lossy().as_ref());
    path_edit.set_on_edit(|siv, content, _| {
        let path = PathBuf::from(content.trim());
        if path.is_dir() {
            populate_file_list(siv, &path);
        } else if let Some(parent) = path.parent().filter(|p| p.is_dir()) {
            populate_file_list(siv, parent);
        }
    });
    let submit_tx = tx.clone();
    path_edit.set_on_submit(move |siv, content| {
        let path = PathBuf::from(content.trim());
        if path.is_dir() {
            populate_file_list(siv, &path);
        } else {
            start_model_import(siv, content, &submit_tx);
        }
    });
    let path_edit = path_edit.with_name("import_path").min_width(70);

    let file_list = SelectView::<FileEntry>::new()
        .h_align(cursive::align::HAlign::Left)
        .with_name("file_list");

    let hint = TextView::new(s.browse_hint).style(Style::from(Color::Rgb(128, 128, 128)));

    let body = LinearLayout::vertical()
        .child(TextView::new(s.import_body))
        .child(DummyView.fixed_height(1))
        .child(
            TextView::new(s.import_path_label).style(Style::from(Color::Light(BaseColor::Yellow))),
        )
        .child(path_edit)
        .child(DummyView.fixed_height(1))
        .child(
            Panel::new(ScrollView::new(file_list).scroll_x(false).fixed_height(14))
                .title(format!(" {} ", s.browse_title)),
        )
        .child(DummyView.fixed_height(1))
        .child(hint);

    let list_tx = tx.clone();
    siv.add_layer(
        Dialog::around(body)
            .title(format!(" {} ", s.import_title))
            .button(s.browse_select, move |siv| {
                let path = siv
                    .call_on_name("import_path", |v: &mut EditView| {
                        v.get_content().to_string()
                    })
                    .unwrap_or_default();
                start_model_import(siv, &path, &tx);
            })
            .button(s.btn_cancel, |sv| {
                sv.pop_layer();
            })
            .min_width(80),
    );

    // Populate the file list with the starting directory
    populate_file_list(siv, &start_dir);

    // Wire up the file list: Enter navigates directories or imports files
    siv.call_on_name("file_list", |v: &mut SelectView<FileEntry>| {
        v.set_on_submit(move |siv, entry: &FileEntry| {
            let path = PathBuf::from(&entry.path);
            match entry.kind {
                FileKind::Parent | FileKind::Directory => {
                    siv.call_on_name("import_path", |v: &mut EditView| {
                        v.set_content(entry.path.as_str());
                    });
                    populate_file_list(siv, &path);
                }
                FileKind::File => {
                    siv.call_on_name("import_path", |v: &mut EditView| {
                        v.set_content(entry.path.as_str());
                    });
                    start_model_import(siv, &entry.path, &list_tx);
                }
            }
        });
    });
}

pub fn show_voice_sample_dialog(siv: &mut Cursive, store_dir: PathBuf) {
    let s = i18n::t();
    let start_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let mut path_edit = EditView::new().content(start_dir.to_string_lossy().as_ref());
    path_edit.set_on_edit(|siv, content, _| {
        let path = PathBuf::from(content.trim());
        if path.is_dir() {
            populate_wav_file_list(siv, &path);
        } else if let Some(parent) = path.parent().filter(|p| p.is_dir()) {
            populate_wav_file_list(siv, parent);
        }
    });
    let submit_store_dir = store_dir.clone();
    path_edit.set_on_submit(move |siv, content| {
        let path = PathBuf::from(content.trim());
        if path.is_dir() {
            populate_wav_file_list(siv, &path);
        } else {
            select_voice_sample(siv, &path, &submit_store_dir);
        }
    });
    let path_edit = path_edit
        .with_name("voice_sample_browser_path")
        .min_width(70);

    let file_list = SelectView::<FileEntry>::new()
        .h_align(cursive::align::HAlign::Left)
        .with_name("voice_sample_file_list");
    let button_store_dir = store_dir.clone();
    siv.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new(s.denoiser_select_audio_body))
                .child(path_edit)
                .child(DummyView.fixed_height(1))
                .child(
                    Panel::new(ScrollView::new(file_list).scroll_x(false).fixed_height(14))
                        .title(format!(" {} ", s.browse_title)),
                )
                .child(DummyView.fixed_height(1))
                .child(TextView::new(s.browse_hint).style(Style::from(Color::Rgb(128, 128, 128)))),
        )
        .title(format!(" {} ", s.denoiser_select_audio_title))
        .button(s.denoiser_select, move |siv| {
            let path = siv
                .call_on_name("voice_sample_browser_path", |v: &mut EditView| {
                    v.get_content().to_string()
                })
                .unwrap_or_default();
            select_voice_sample(siv, Path::new(path.trim()), &button_store_dir);
        })
        .button(i18n::t().denoiser_cancel, |siv| {
            siv.pop_layer();
        })
        .min_width(80),
    );

    populate_wav_file_list(siv, &start_dir);

    siv.call_on_name("voice_sample_file_list", |v: &mut SelectView<FileEntry>| {
        let list_store_dir = store_dir.clone();
        v.set_on_submit(move |siv, entry: &FileEntry| {
            let path = PathBuf::from(&entry.path);
            match entry.kind {
                FileKind::Parent | FileKind::Directory => {
                    siv.call_on_name("voice_sample_browser_path", |v: &mut EditView| {
                        v.set_content(entry.path.as_str());
                    });
                    populate_wav_file_list(siv, &path);
                }
                FileKind::File => {
                    siv.call_on_name("voice_sample_browser_path", |v: &mut EditView| {
                        v.set_content(entry.path.as_str());
                    });
                    select_voice_sample(siv, &path, &list_store_dir);
                }
            }
        });
    });
}

fn select_voice_sample(siv: &mut Cursive, path: &Path, store_dir: &Path) {
    let s = i18n::t();
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
    {
        siv.add_layer(
            Dialog::text(s.denoiser_wav_error).button(s.denoiser_ok, |sv| {
                sv.pop_layer();
            }),
        );
        return;
    }
    match crate::model::import::import_voice_sample(path, store_dir) {
        Ok(imported) => {
            siv.call_on_name("voxlock_voice_sample_path", |v: &mut TextView| {
                v.set_content(imported.to_string_lossy().as_ref());
            });
            siv.pop_layer();
        }
        Err(e) => {
            siv.add_layer(
                Dialog::text(s.voice_import_failed_fmt.replace("{}", &e.to_string()))
                    .title(format!(" {} ", s.denoiser_select_audio_title))
                    .button(s.denoiser_ok, |sv| {
                        sv.pop_layer();
                    }),
            );
        }
    }
}

fn human_size(size: u64) -> String {
    if size > 1_000_000_000 {
        format!("{}G", size / 1_000_000_000)
    } else if size > 1_000_000 {
        format!("{}M", size / 1_000_000)
    } else if size > 1000 {
        format!("{}K", size / 1000)
    } else {
        format!("{size}")
    }
}

fn populate_wav_file_list(siv: &mut Cursive, dir: &Path) {
    let s = i18n::t();
    let mut entries = Vec::new();
    if let Some(parent) = dir.parent() {
        entries.push(FileEntry {
            name: s.browse_go_up.to_string(),
            path: parent.to_string_lossy().into_owned(),
            kind: FileKind::Parent,
        });
    }
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in read_dir.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                dirs.push(FileEntry {
                    name: format!("{name}/"),
                    path: path.to_string_lossy().into_owned(),
                    kind: FileKind::Directory,
                });
                continue;
            }
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
            {
                continue;
            }
            let size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
            files.push(FileEntry {
                name: format!("{name}  ({})", human_size(size)),
                path: path.to_string_lossy().into_owned(),
                kind: FileKind::File,
            });
        }
        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        entries.extend(dirs);
        entries.extend(files);
    }
    siv.call_on_name("voice_sample_file_list", |v: &mut SelectView<FileEntry>| {
        v.clear();
        if entries.is_empty() {
            v.add_item(
                s.browse_dir_empty.to_string(),
                FileEntry {
                    name: String::new(),
                    path: String::new(),
                    kind: FileKind::File,
                },
            );
        } else {
            for entry in entries {
                v.add_item(entry.name.clone(), entry);
            }
        }
    });
}

#[derive(Clone, Debug)]
struct FileEntry {
    name: String,
    path: String,
    kind: FileKind,
}

#[derive(Clone, Debug)]
enum FileKind {
    Parent,
    Directory,
    File,
}

fn populate_file_list(siv: &mut Cursive, dir: &std::path::Path) {
    let s = i18n::t();
    let mut entries: Vec<FileEntry> = Vec::new();

    // Add ".." parent entry if not root
    if let Some(parent) = dir.parent() {
        entries.push(FileEntry {
            name: s.browse_go_up.to_string(),
            path: parent.to_string_lossy().into_owned(),
            kind: FileKind::Parent,
        });
    }

    // Read directory contents
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name != ".." {
                continue; // skip hidden files
            }
            let path = entry.path().to_string_lossy().into_owned();
            let ft = entry.file_type().ok();
            if ft.as_ref().map(|t| t.is_dir()).unwrap_or(false) {
                dirs.push(FileEntry {
                    name: format!("{}/", name),
                    path,
                    kind: FileKind::Directory,
                });
            } else {
                let size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
                let size_str = if size > 1_000_000_000 {
                    format!("{}G", size / 1_000_000_000)
                } else if size > 1_000_000 {
                    format!("{}M", size / 1_000_000)
                } else if size > 1000 {
                    format!("{}K", size / 1000)
                } else {
                    format!("{}", size)
                };
                files.push(FileEntry {
                    name: format!("{}  ({})", name, size_str),
                    path,
                    kind: FileKind::File,
                });
            }
        }
        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        entries.extend(dirs);
        entries.extend(files);
    }

    siv.call_on_name("file_list", |v: &mut SelectView<FileEntry>| {
        v.clear();
        if entries.is_empty() {
            v.add_item(
                s.browse_dir_empty.to_string(),
                FileEntry {
                    name: String::new(),
                    path: String::new(),
                    kind: FileKind::File,
                },
            );
        } else {
            for entry in entries {
                v.add_item(entry.name.clone(), entry);
            }
        }
    });
}

pub fn show_network_dialog(
    siv: &mut Cursive,
    app: Arc<Mutex<App>>,
    cmd_tx: mpsc::Sender<TuiCommand>,
) {
    let s = i18n::t();
    let (token_set, hosts_enabled, hosts_count, hosts_source, mirror_count, silence_ms) = {
        let a = app.lock().unwrap();
        (
            a.config
                .github_token
                .as_ref()
                .map(|t| !t.is_empty())
                .unwrap_or(false),
            a.config.hosts_enabled,
            a.config.hosts.len(),
            a.config.hosts_source_url.clone(),
            a.config.mirrors.len(),
            a.config.silence_ms,
        )
    };

    let dim = Style::from(Color::Rgb(128, 128, 128));
    let val = Style::from(Color::Light(BaseColor::White));
    let mut summary = StyledString::new();
    summary.append_styled(s.net_summary_header, dim);
    summary.append_styled(s.net_token_label, dim);
    summary.append_styled(if token_set { s.net_set } else { s.net_none }, val);
    summary.append_plain("\n");
    summary.append_styled(s.net_hosts_label, dim);
    summary.append_styled(
        format!(
            "{}{}",
            if hosts_enabled {
                s.net_enabled
            } else {
                s.net_disabled
            },
            s.net_hosts_entries_fmt
                .replace("{}", &hosts_count.to_string())
        ),
        val,
    );
    summary.append_plain("\n");
    summary.append_styled(s.net_hosts_source_label, dim);
    summary.append_styled(
        if hosts_source.is_empty() {
            s.net_none.to_string()
        } else {
            hosts_source
        },
        val,
    );
    summary.append_plain("\n");
    summary.append_styled(s.net_mirror_label, dim);
    summary.append_styled(format!("{mirror_count}"), val);
    summary.append_plain("\n");
    summary.append_styled(s.net_silence_label, dim);
    summary.append_styled(format!("{} ms", silence_ms), val);
    summary.append_plain("\n");
    summary.append_styled(s.net_lang_label, dim);
    summary.append_styled(i18n::current().label(), val);

    let mut select = SelectView::<usize>::new().autojump();
    select.add_item(s.net_action_set_token, 0);
    select.add_item(s.net_action_clear_token, 1);
    select.add_item(s.net_action_set_hosts_url, 2);
    select.add_item(
        if hosts_enabled {
            s.net_action_toggle_hosts_on
        } else {
            s.net_action_toggle_hosts_off
        },
        3,
    );
    select.add_item(s.net_action_pull_hosts, 4);
    select.add_item(s.net_action_add_mirror, 5);
    select.add_item(s.net_action_list_mirrors, 6);
    select.add_item(s.net_action_clear_cache, 7);
    select.add_item(s.net_action_set_silence, 8);

    let tx = cmd_tx;
    let app_inner = app.clone();
    select.set_on_submit(move |siv, &idx| {
        siv.pop_layer();
        match idx {
            0 => show_github_token_dialog(siv, tx.clone()),
            1 => {
                let _ = tx.try_send(TuiCommand::SetGithubToken(String::new()));
            }
            2 => show_hosts_source_dialog(siv, tx.clone()),
            3 => {
                let _ = tx.try_send(TuiCommand::ToggleHosts);
            }
            4 => {
                let _ = tx.try_send(TuiCommand::UpdateHosts);
            }
            5 => show_add_mirror_dialog(siv, tx.clone()),
            6 => show_mirror_list_dialog(siv, app_inner.clone(), tx.clone()),
            7 => show_clear_cache_dialog(siv, app_inner.clone()),
            8 => show_silence_dialog(siv, silence_ms, tx.clone()),
            _ => {}
        }
    });

    siv.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new(summary))
                .child(DummyView.fixed_height(1))
                .child(select),
        )
        .title(format!(" {} ", s.net_title))
        .button(s.btn_close, |sv| {
            sv.pop_layer();
        })
        .min_width(64),
    );
}

fn show_github_token_dialog(siv: &mut Cursive, cmd_tx: mpsc::Sender<TuiCommand>) {
    let s = i18n::t();
    let tx = cmd_tx;
    siv.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new(s.token_body))
                .child(DummyView.fixed_height(1))
                .child(
                    EditView::new()
                        .secret()
                        .with_name("token_input")
                        .min_width(60),
                ),
        )
        .title(format!(" {} ", s.token_title))
        .button(s.btn_save, move |siv| {
            let token = siv
                .call_on_name("token_input", |v: &mut EditView| v.get_content())
                .unwrap_or_default()
                .trim()
                .to_string();
            let _ = tx.try_send(TuiCommand::SetGithubToken(token));
            siv.pop_layer();
        })
        .button(s.btn_cancel, |sv| {
            sv.pop_layer();
        })
        .min_width(72),
    );
}

fn show_silence_dialog(siv: &mut Cursive, current_ms: i32, cmd_tx: mpsc::Sender<TuiCommand>) {
    let s = i18n::t();
    let tx = cmd_tx;
    siv.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new(s.silence_body))
                .child(DummyView.fixed_height(1))
                .child(
                    EditView::new()
                        .content(format!("{current_ms}"))
                        .with_name("silence_input")
                        .min_width(10),
                ),
        )
        .title(format!(" {} ", s.silence_title))
        .button(s.btn_save, move |siv| {
            let input = siv
                .call_on_name("silence_input", |v: &mut EditView| v.get_content())
                .unwrap_or_default()
                .trim()
                .to_string();
            if let Ok(ms) = input.parse::<i32>() {
                let clamped = ms.clamp(500, 5000);
                let _ = tx.try_send(TuiCommand::SetSilenceMs(clamped));
            }
            siv.pop_layer();
        })
        .button(s.btn_cancel, |sv| {
            sv.pop_layer();
        })
        .min_width(60),
    );
}

fn show_hosts_source_dialog(siv: &mut Cursive, cmd_tx: mpsc::Sender<TuiCommand>) {
    let s = i18n::t();
    let tx = cmd_tx;
    siv.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new(s.hosts_src_body))
                .child(DummyView.fixed_height(1))
                .child(EditView::new().with_name("hosts_url_input").min_width(60)),
        )
        .title(format!(" {} ", s.hosts_src_title))
        .button(s.btn_save, move |siv| {
            let url = siv
                .call_on_name("hosts_url_input", |v: &mut EditView| v.get_content())
                .unwrap_or_default()
                .trim()
                .to_string();
            let _ = tx.try_send(TuiCommand::SetHostsSource(url));
            siv.pop_layer();
        })
        .button(s.btn_cancel, |sv| {
            sv.pop_layer();
        })
        .min_width(72),
    );
}

fn show_add_mirror_dialog(siv: &mut Cursive, cmd_tx: mpsc::Sender<TuiCommand>) {
    let s = i18n::t();
    let tx = cmd_tx;
    let body = LinearLayout::vertical()
        .child(TextView::new(s.mirror_add_body))
        .child(DummyView.fixed_height(1))
        .child(TextView::new(s.mirror_pattern_label))
        .child(EditView::new().with_name("mirror_pattern").min_width(60))
        .child(DummyView.fixed_height(1))
        .child(TextView::new(s.mirror_replacement_label))
        .child(
            EditView::new()
                .with_name("mirror_replacement")
                .min_width(60),
        );

    siv.add_layer(
        Dialog::around(body)
            .title(format!(" {} ", s.mirror_add_title))
            .button(s.btn_add, move |siv| {
                let pattern = siv
                    .call_on_name("mirror_pattern", |v: &mut EditView| v.get_content())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let replacement = siv
                    .call_on_name("mirror_replacement", |v: &mut EditView| v.get_content())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if pattern.is_empty() {
                    let s = i18n::t();
                    siv.add_layer(Dialog::text(s.mirror_pattern_empty).button(s.btn_ok, |sv| {
                        sv.pop_layer();
                    }));
                    return;
                }
                let _ = tx.try_send(TuiCommand::AddMirror {
                    pattern,
                    replacement,
                });
                siv.pop_layer();
            })
            .button(s.btn_cancel, |sv| {
                sv.pop_layer();
            })
            .min_width(72),
    );
}

fn show_mirror_list_dialog(
    siv: &mut Cursive,
    app: Arc<Mutex<App>>,
    cmd_tx: mpsc::Sender<TuiCommand>,
) {
    let s = i18n::t();
    let mirrors: Vec<(usize, String, String, bool)> = {
        let a = app.lock().unwrap();
        a.config
            .mirrors
            .iter()
            .enumerate()
            .map(|(i, m)| (i, m.pattern.clone(), m.replacement.clone(), m.enabled))
            .collect()
    };

    if mirrors.is_empty() {
        siv.add_layer(
            Dialog::text(s.mirror_list_empty)
                .title(format!(" {} ", s.mirror_list_title))
                .button(s.btn_close, |sv| {
                    sv.pop_layer();
                }),
        );
        return;
    }

    let mut select = SelectView::<usize>::new().autojump();
    for (i, pat, rep, enabled) in &mirrors {
        let label = format!(
            "[{}]  {}  ->  {}",
            if *enabled { "ON " } else { "OFF" },
            pat,
            rep,
        );
        select.add_item(label, *i);
    }

    let tx_submit = cmd_tx.clone();
    select.set_on_submit(move |siv, &idx| {
        let _ = tx_submit.try_send(TuiCommand::ToggleMirror(idx));
        siv.pop_layer();
    });

    let tx_clear = cmd_tx.clone();
    let body = LinearLayout::vertical()
        .child(TextView::new(s.mirror_list_hint).style(Style::from(Color::Rgb(128, 128, 128))))
        .child(DummyView.fixed_height(1))
        .child(select.min_height(6).max_height(14));

    siv.add_layer(
        Dialog::around(ScrollView::new(body))
            .title(format!(" {} ", s.mirror_list_title))
            .button(s.btn_clear_all, move |sv| {
                let _ = tx_clear.try_send(TuiCommand::ClearMirrors);
                sv.pop_layer();
            })
            .button(s.btn_close, |sv| {
                sv.pop_layer();
            })
            .min_width(72),
    );
}

fn show_clear_cache_dialog(siv: &mut Cursive, app: Arc<Mutex<App>>) {
    let result = {
        let a = app.lock().unwrap();
        download::clear_release_cache(&a.store_dir)
    };
    let s = i18n::t();
    let msg = match result {
        Ok(()) => s.cache_cleared.to_string(),
        Err(e) => s.cache_failed_fmt.replace("{}", &e.to_string()),
    };
    siv.add_layer(Dialog::text(msg).button(s.btn_ok, |sv| {
        sv.pop_layer();
    }));
}

pub fn show_language_dialog(
    siv: &mut Cursive,
    app: Arc<Mutex<App>>,
    cmd_tx: mpsc::Sender<TuiCommand>,
) {
    let s = i18n::t();
    let mut select = SelectView::<String>::new().autojump();
    let current = i18n::current();
    let mut current_pos = 0usize;
    for (i, lang) in i18n::Lang::all().iter().enumerate() {
        let marker = if *lang == current { " *" } else { "" };
        select.add_item(format!("{}{marker}", lang.label()), lang.code().to_string());
        if *lang == current {
            current_pos = i;
        }
    }
    select.set_selection(current_pos);

    let app_inner = app.clone();
    let cmd_tx_inner = cmd_tx;
    select.set_on_submit(move |siv, code: &String| {
        siv.pop_layer();
        let lang = i18n::Lang::from_code(code).unwrap_or(i18n::Lang::En);
        if lang == i18n::current() {
            return;
        }
        i18n::set_lang(lang);
        {
            let mut a = app_inner.lock().unwrap();
            a.config.language = Some(lang.code().to_string());
            let _ = a.config.save(&a.store_dir);
        }
        super::install_main_layout(siv, &app_inner, &cmd_tx_inner);
        tracing::info!("Interface language switched to {}", lang.code());
    });

    siv.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(TextView::new(s.lang_body))
                .child(DummyView.fixed_height(1))
                .child(select),
        )
        .title(format!(" {} ", s.lang_title))
        .button(s.btn_close, |sv| {
            sv.pop_layer();
        })
        .min_width(48),
    );
}

pub fn show_translation_settings(
    siv: &mut Cursive,
    app: Arc<Mutex<App>>,
    cmd_tx: mpsc::Sender<TuiCommand>,
) {
    let s = i18n::t();
    let (mode, target, triggers, strip, model_path) = {
        let a = app.lock().unwrap();
        let mode = a.config.translation_mode.clone();
        let target = a.config.translation_target.clone();
        let triggers = a.config.translation_triggers.clone();
        let strip = a.config.translation_strip_hints;
        let model = a.config.translation_model.clone().unwrap_or_default();
        (mode, target, triggers, strip, model)
    };

    use std::sync::{Arc as StdArc, Mutex as StdMutex};

    let selected_mode = StdArc::new(StdMutex::new(mode));
    let selected_target = StdArc::new(StdMutex::new(target));
    let entered_triggers = StdArc::new(StdMutex::new(triggers));
    let strip_flag = StdArc::new(StdMutex::new(strip));

    let mut mode_select = SelectView::new();
    mode_select.add_all_str(["always", "auto"]);
    mode_select.set_selection(if *selected_mode.lock().unwrap() == "auto" {
        1
    } else {
        0
    });
    {
        let sm = selected_mode.clone();
        mode_select.set_on_select(move |_siv, item: &String| {
            *sm.lock().unwrap() = item.clone();
        });
    }

    let target_langs = [
        ("en", "英文 English"),
        ("zh", "中文 Chinese"),
        ("ja", "日文 Japanese"),
        ("ko", "韩文 Korean"),
        ("fr", "法文 French"),
        ("de", "德文 German"),
        ("es", "西班牙文 Spanish"),
        ("ru", "俄文 Russian"),
        ("pt", "葡萄牙文 Portuguese"),
        ("it", "意大利文 Italian"),
        ("th", "泰文 Thai"),
        ("vi", "越南文 Vietnamese"),
        ("ar", "阿拉伯文 Arabic"),
    ];
    let mut target_select = SelectView::new();
    for (code, label) in &target_langs {
        target_select.add_item(*label, code.to_string());
    }
    target_select.set_selection(
        target_langs
            .iter()
            .position(|(c, _)| *c == *selected_target.lock().unwrap())
            .unwrap_or(0),
    );
    {
        let st = selected_target.clone();
        target_select.set_on_select(move |_siv, item: &String| {
            let code = target_langs
                .iter()
                .find(|(_, l)| *l == item.as_str())
                .map(|(c, _)| *c)
                .unwrap_or("en");
            *st.lock().unwrap() = code.to_string();
        });
    }

    let mut triggers_edit = EditView::new().content(entered_triggers.lock().unwrap().as_str());
    {
        let et = entered_triggers.clone();
        triggers_edit.set_on_edit(move |_siv, content: &str, _cursor| {
            *et.lock().unwrap() = content.to_string();
        });
    }
    let triggers_edit = triggers_edit.min_width(40);

    let mut strip_checkbox = Checkbox::new().with_checked(strip);
    {
        let sf = strip_flag.clone();
        strip_checkbox.set_on_change(move |_siv, checked| {
            *sf.lock().unwrap() = checked;
        });
    }

    let model_label = format!(
        "Model: {}",
        if model_path.is_empty() {
            "(none)"
        } else {
            &model_path
        }
    );

    let body = LinearLayout::vertical()
        .child(TextView::new(s.trset_mode_label))
        .child(mode_select.min_width(20))
        .child(DummyView.fixed_height(1))
        .child(TextView::new(s.trset_target_label))
        .child(target_select.min_width(20))
        .child(DummyView.fixed_height(1))
        .child(TextView::new(s.trset_triggers_label))
        .child(triggers_edit)
        .child(DummyView.fixed_height(1))
        .child(
            LinearLayout::horizontal()
                .child(strip_checkbox)
                .child(TextView::new(format!(" {}", s.trset_strip_label))),
        )
        .child(DummyView.fixed_height(1))
        .child(
            TextView::new(model_label)
                .no_wrap()
                .with_name("trset_model_label"),
        )
        .child(DummyView.fixed_height(1))
        .child(LinearLayout::horizontal().child({
            let app_browse = app.clone();
            let tx_browse = cmd_tx.clone();
            cursive::views::Button::new(s.browse_title, move |siv| {
                show_gguf_browser(siv, app_browse.clone(), tx_browse.clone());
            })
        }));

    let app_c = app.clone();
    let cmd_tx_save = cmd_tx.clone();
    siv.add_layer(
        Dialog::around(body)
            .title(format!(" {} ", s.trset_title))
            .button(s.btn_save, move |sv| {
                let mode = selected_mode.lock().unwrap().clone();
                let target = selected_target.lock().unwrap().clone();
                let triggers = entered_triggers.lock().unwrap().clone();
                let strip = *strip_flag.lock().unwrap();
                {
                    let mut a = app_c.lock().unwrap();
                    a.config.translation_mode = mode;
                    a.config.translation_target = target;
                    a.config.translation_triggers = triggers;
                    a.config.translation_strip_hints = strip;
                    let _ = a.config.save(&a.store_dir);
                }
                let _ = cmd_tx_save.try_send(TuiCommand::ReloadTranslation);
                let s = i18n::t();
                sv.add_layer(
                    Dialog::text(s.trset_saved)
                        .title(format!(" {} ", s.trset_title))
                        .button(s.btn_close, |sv| {
                            sv.pop_layer();
                            sv.pop_layer();
                        }),
                );
            })
            .button(s.btn_cancel, |sv| {
                sv.pop_layer();
            })
            .min_width(56),
    );
}

fn aux_kind_label(s: &i18n::Strings, kind: AuxKind) -> &'static str {
    match kind {
        AuxKind::Vad => s.aux_kind_vad,
        AuxKind::Punctuation => s.aux_kind_punctuation,
        AuxKind::Denoiser => s.aux_kind_denoiser,
        AuxKind::Speaker => s.aux_kind_speaker,
        AuxKind::Translation => s.aux_kind_translation,
    }
}

fn confirm_delete_aux(siv: &mut Cursive, model: &AuxModel, cmd_tx: mpsc::Sender<TuiCommand>) {
    let s = i18n::t();
    let body = s.confirm_delete_aux_body_fmt.replace("{}", &model.name);
    let dir = model.dir.to_string_lossy().into_owned();
    siv.add_layer(
        Dialog::text(body)
            .title(format!(" {} ", s.confirm_delete_title))
            .button(s.btn_delete, move |sv| {
                let _ = cmd_tx.try_send(TuiCommand::DeleteAuxModel(dir.clone()));
                sv.pop_layer();
                sv.pop_layer();
            })
            .button(s.btn_cancel, |sv| {
                sv.pop_layer();
            }),
    );
}

pub fn show_aux_models_dialog(
    siv: &mut Cursive,
    app: Arc<Mutex<App>>,
    cmd_tx: mpsc::Sender<TuiCommand>,
) {
    let s = i18n::t();
    let models = app.lock().unwrap().list_aux_models();
    if models.is_empty() {
        siv.add_layer(
            Dialog::text(s.aux_models_none)
                .title(format!(" {} ", s.aux_models_title))
                .button(s.btn_close, |sv| {
                    sv.pop_layer();
                }),
        );
        return;
    }

    let mut list = SelectView::<AuxModel>::new().h_align(cursive::align::HAlign::Left);
    for model in &models {
        let label = format!("[{}] {}", aux_kind_label(s, model.kind), model.name);
        list.add_item(label, model.clone());
    }
    let tx_submit = cmd_tx.clone();
    list.set_on_submit(move |siv, model: &AuxModel| {
        confirm_delete_aux(siv, model, tx_submit.clone());
    });

    let tx_button = cmd_tx.clone();
    siv.add_layer(
        Dialog::around(
            LinearLayout::vertical()
                .child(
                    TextView::new(s.aux_models_hint).style(Style::from(Color::Rgb(128, 128, 128))),
                )
                .child(
                    ScrollView::new(list.with_name("aux_model_list"))
                        .scroll_x(false)
                        .fixed_height(12),
                ),
        )
        .title(format!(" {} ", s.aux_models_title))
        .button(s.btn_delete, move |siv| {
            let selected = siv
                .call_on_name("aux_model_list", |v: &mut SelectView<AuxModel>| {
                    v.selection().map(|m| (*m).clone())
                })
                .flatten();
            if let Some(model) = selected {
                confirm_delete_aux(siv, &model, tx_button.clone());
            }
        })
        .button(s.btn_close, |sv| {
            sv.pop_layer();
        })
        .min_width(60),
    );
}

fn show_gguf_browser(siv: &mut Cursive, app: Arc<Mutex<App>>, cmd_tx: mpsc::Sender<TuiCommand>) {
    let s = i18n::t();
    let models_dir = {
        let a = app.lock().unwrap();
        crate::model::models_dir(&a.store_dir)
    };
    let start_dir = if models_dir.exists() {
        models_dir
    } else {
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"))
    };

    let file_list = SelectView::<FileEntry>::new()
        .h_align(cursive::align::HAlign::Left)
        .with_name("gguf_file_list");

    let path_label = TextView::new(start_dir.to_string_lossy().as_ref())
        .style(Style::from(Color::Rgb(128, 128, 128)))
        .with_name("gguf_current_path");

    let hint = TextView::new(s.browse_hint).style(Style::from(Color::Rgb(128, 128, 128)));

    let body = LinearLayout::vertical()
        .child(path_label)
        .child(DummyView.fixed_height(1))
        .child(
            Panel::new(ScrollView::new(file_list).scroll_x(false).fixed_height(16))
                .title(format!(" {} ", s.browse_title)),
        )
        .child(DummyView.fixed_height(1))
        .child(hint);

    siv.add_layer(
        Dialog::around(body)
            .title(format!(" {} ", s.browse_title))
            .button(s.btn_cancel, |sv| {
                sv.pop_layer();
            })
            .min_width(80),
    );

    populate_gguf_file_list(siv, &start_dir);

    let app_inner = app.clone();
    siv.call_on_name("gguf_file_list", |v: &mut SelectView<FileEntry>| {
        v.set_on_submit(move |siv, entry: &FileEntry| {
            let entry = entry.clone();
            match entry.kind {
                FileKind::Parent => {
                    let current = siv
                        .call_on_name("gguf_current_path", |v: &mut TextView| {
                            v.get_content().source().to_string()
                        })
                        .unwrap_or_default();
                    let current_path = std::path::PathBuf::from(current.trim());
                    let parent = current_path
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or(current_path);
                    siv.call_on_name("gguf_current_path", |v: &mut TextView| {
                        v.set_content(parent.to_string_lossy().as_ref());
                    });
                    populate_gguf_file_list(siv, &parent);
                }
                FileKind::Directory => {
                    let path = std::path::PathBuf::from(&entry.path);
                    siv.call_on_name("gguf_current_path", |v: &mut TextView| {
                        v.set_content(entry.path.as_str());
                    });
                    populate_gguf_file_list(siv, &path);
                }
                FileKind::File => {
                    // Selected a .gguf file — update config and close
                    let gguf_path = entry.path.clone();
                    {
                        let mut a = app_inner.lock().unwrap();
                        a.config.translation_model = Some(gguf_path.clone());
                        a.config.translation_enabled = true;
                        let _ = a.config.save(&a.store_dir);
                    }
                    let _ = cmd_tx.try_send(TuiCommand::ReloadTranslation);
                    // Update the model label in the parent translation settings dialog
                    let display_path =
                        crate::path_display::for_log_path(&std::path::PathBuf::from(&gguf_path));
                    siv.call_on_name("trset_model_label", |v: &mut TextView| {
                        v.set_content(format!("Model: {display_path}"));
                    });
                    siv.pop_layer(); // close browser
                }
            }
        });
    });
}

fn populate_gguf_file_list(siv: &mut Cursive, dir: &std::path::Path) {
    let s = i18n::t();
    let mut entries: Vec<FileEntry> = Vec::new();

    if dir.parent().is_some() {
        entries.push(FileEntry {
            name: s.browse_go_up.to_string(),
            path: String::new(),
            kind: FileKind::Parent,
        });
    }

    if let Ok(read_dir) = std::fs::read_dir(dir) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name != ".." {
                continue;
            }
            let path = entry.path().to_string_lossy().into_owned();
            let ft = entry.file_type().ok();
            if ft.as_ref().map(|t| t.is_dir()).unwrap_or(false) {
                dirs.push(FileEntry {
                    name: format!("{}/", name),
                    path,
                    kind: FileKind::Directory,
                });
            } else {
                let ext = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .unwrap_or_default();
                if ext != "gguf" {
                    continue;
                }
                let size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
                let size_str = if size > 1_000_000_000 {
                    format!("{}G", size / 1_000_000_000)
                } else if size > 1_000_000 {
                    format!("{}M", size / 1_000_000)
                } else if size > 1000 {
                    format!("{}K", size / 1000)
                } else {
                    format!("{}", size)
                };
                files.push(FileEntry {
                    name: format!("{}  ({})", name, size_str),
                    path,
                    kind: FileKind::File,
                });
            }
        }
        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        entries.extend(dirs);
        entries.extend(files);
    }

    siv.call_on_name("gguf_file_list", |v: &mut SelectView<FileEntry>| {
        v.clear();
        if entries.is_empty() {
            v.add_item(
                s.browse_dir_empty.to_string(),
                FileEntry {
                    name: String::new(),
                    path: String::new(),
                    kind: FileKind::File,
                },
            );
        } else {
            for entry in entries {
                v.add_item(entry.name.clone(), entry);
            }
        }
    });
}
