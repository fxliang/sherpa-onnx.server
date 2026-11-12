use std::sync::RwLock;

mod de;
mod es;
mod fr;
mod ja;
mod ko;
mod pt_br;
mod ru;

use de::DE;
use es::ES;
use fr::FR;
use ja::JA;
use ko::KO;
use pt_br::PT_BR;
use ru::RU;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhCn,
    ZhTw,
    Ja,
    Ko,
    Ru,
    Es,
    PtBr,
    Fr,
    De,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::ZhCn => "zh-CN",
            Lang::ZhTw => "zh-TW",
            Lang::Ja => "ja",
            Lang::Ko => "ko",
            Lang::Ru => "ru",
            Lang::Es => "es",
            Lang::PtBr => "pt-BR",
            Lang::Fr => "fr",
            Lang::De => "de",
        }
    }

    pub fn from_code(s: &str) -> Option<Self> {
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        match lower.as_str() {
            "en" | "en-us" | "en_us" | "english" => Some(Lang::En),
            "zh" | "zh-cn" | "zh_cn" | "zh-hans" | "zh_hans" | "cn" => Some(Lang::ZhCn),
            "zh-tw" | "zh_tw" | "zh-hk" | "zh_hk" | "zh-hant" | "zh_hant" | "tw" | "hk" => {
                Some(Lang::ZhTw)
            }
            "ja" | "ja-jp" | "ja_jp" | "jp" | "japanese" => Some(Lang::Ja),
            "ko" | "ko-kr" | "ko_kr" | "kr" | "korean" => Some(Lang::Ko),
            "ru" | "ru-ru" | "ru_ru" | "russian" => Some(Lang::Ru),
            "es" | "es-es" | "es_es" | "es-419" | "es_419" | "es-mx" | "es_mx" | "spanish" => {
                Some(Lang::Es)
            }
            "pt" | "pt-br" | "pt_br" | "pt-pt" | "pt_pt" | "portuguese" | "brazilian" => {
                Some(Lang::PtBr)
            }
            "fr" | "fr-fr" | "fr_fr" | "french" => Some(Lang::Fr),
            "de" | "de-de" | "de_de" | "german" => Some(Lang::De),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::ZhCn => "简体中文",
            Lang::ZhTw => "繁體中文",
            Lang::Ja => "日本語",
            Lang::Ko => "한국어",
            Lang::Ru => "Русский",
            Lang::Es => "Español",
            Lang::PtBr => "Português (Brasil)",
            Lang::Fr => "Français",
            Lang::De => "Deutsch",
        }
    }

    pub fn all() -> &'static [Lang] {
        &[
            Lang::En,
            Lang::ZhCn,
            Lang::ZhTw,
            Lang::Ja,
            Lang::Ko,
            Lang::Ru,
            Lang::Es,
            Lang::PtBr,
            Lang::Fr,
            Lang::De,
        ]
    }
}

pub fn detect_default_lang() -> Lang {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(v) = std::env::var(var) {
            let raw = v
                .split(['.', ':'])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if raw.is_empty() || raw == "c" || raw == "posix" {
                continue;
            }
            if let Some(lang) = Lang::from_code(&raw) {
                return lang;
            }
            let base = raw.split(['_', '-']).next().unwrap_or("");
            if let Some(lang) = Lang::from_code(base) {
                return lang;
            }
        }
    }
    Lang::En
}

static CURRENT: RwLock<Lang> = RwLock::new(Lang::En);

pub fn set_lang(lang: Lang) {
    if let Ok(mut w) = CURRENT.write() {
        *w = lang;
    }
}

pub fn current() -> Lang {
    CURRENT.read().map(|g| *g).unwrap_or(Lang::En)
}

pub fn t() -> &'static Strings {
    match current() {
        Lang::En => &EN,
        Lang::ZhCn => &ZH_CN,
        Lang::ZhTw => &ZH_TW,
        Lang::Ja => &JA,
        Lang::Ko => &KO,
        Lang::Ru => &RU,
        Lang::Es => &ES,
        Lang::PtBr => &PT_BR,
        Lang::Fr => &FR,
        Lang::De => &DE,
    }
}

pub struct Strings {
    pub title_models: &'static str,
    pub title_log: &'static str,
    pub title_status: &'static str,
    pub title_shortcuts: &'static str,
    pub model_panel_hint: &'static str,
    pub log_panel_hint: &'static str,

    pub help_kw_models: &'static str,
    pub help_kw_server: &'static str,
    pub help_kw_download: &'static str,
    pub help_kw_import: &'static str,
    pub help_kw_select: &'static str,
    pub help_kw_delete: &'static str,
    pub help_kw_run_stop: &'static str,
    pub help_kw_clear_cache: &'static str,
    pub help_kw_help: &'static str,
    pub help_kw_quit: &'static str,
    pub help_kw_settings: &'static str,

    pub btn_quit: &'static str,
    pub btn_cancel: &'static str,
    pub btn_ok: &'static str,
    pub btn_delete: &'static str,
    pub btn_save: &'static str,
    pub btn_add: &'static str,
    pub btn_close: &'static str,
    pub btn_refresh: &'static str,
    pub btn_clear_all: &'static str,
    pub btn_download: &'static str,

    pub confirm_quit_title: &'static str,
    pub confirm_quit_body: &'static str,

    pub confirm_delete_title: &'static str,
    /// Format string with one `{}` placeholder for the model number (1-based).
    pub confirm_delete_body_fmt: &'static str,

    pub cache_title: &'static str,
    pub cache_cleared: &'static str,
    /// Format string with one `{}` placeholder for the error message.
    pub cache_failed_fmt: &'static str,

    pub trset_title: &'static str,
    pub trset_mode_label: &'static str,
    pub trset_target_label: &'static str,
    pub trset_triggers_label: &'static str,
    pub trset_strip_label: &'static str,
    pub trset_saved: &'static str,

    pub status_ip: &'static str,
    pub status_models_dir: &'static str,
    pub status_discovered: &'static str,
    pub status_active_asr: &'static str,
    pub status_vad: &'static str,
    pub status_punctuation: &'static str,
    pub status_websocket: &'static str,
    pub status_running: &'static str,
    pub status_stopped: &'static str,
    pub status_mode_streaming: &'static str,
    pub status_mode_offline: &'static str,
    /// Format string with `{}` for count.
    pub status_models_count_fmt: &'static str,
    pub status_none: &'static str,
    pub status_unavailable: &'static str,
    pub status_no_models_hint: &'static str,
    pub status_no_active_hint: &'static str,

    pub list_mode_stream: &'static str,
    pub list_mode_offline: &'static str,

    pub help_title: &'static str,
    pub help_section_getting_started: &'static str,
    pub help_getting_started_1_a: &'static str,
    pub help_getting_started_1_b: &'static str,
    pub help_getting_started_1_c: &'static str,
    pub help_getting_started_2_a: &'static str,
    pub help_getting_started_2_b: &'static str,
    pub help_getting_started_3_a: &'static str,
    pub help_getting_started_3_b: &'static str,
    pub help_section_keys: &'static str,
    pub help_key_d: &'static str,
    pub help_key_i: &'static str,
    pub help_key_s: &'static str,
    pub help_key_enter: &'static str,
    pub help_key_x: &'static str,
    pub help_key_r: &'static str,
    pub help_key_o: &'static str,
    pub help_key_c: &'static str,
    pub help_key_q: &'static str,
    pub help_key_help: &'static str,
    pub help_section_files: &'static str,
    pub help_files_default: &'static str,
    pub help_files_models: &'static str,
    pub help_files_punct: &'static str,
    pub help_files_config: &'static str,
    pub help_files_cache: &'static str,
    pub help_section_network: &'static str,
    pub help_network_press_a: &'static str,
    pub help_network_press_b: &'static str,
    pub help_network_token: &'static str,
    pub help_network_mirror: &'static str,
    pub help_network_mirror_example: &'static str,
    pub help_network_hosts_a: &'static str,
    pub help_network_hosts_b: &'static str,
    pub help_section_models: &'static str,
    pub help_models_list_1: &'static str,
    pub help_models_list_2: &'static str,
    pub help_models_aux_1: &'static str,
    pub help_models_aux_2: &'static str,

    pub dl_loading: &'static str,
    pub dl_title: &'static str,
    pub dl_fetch_failed_title: &'static str,
    /// Format with `{}` for the underlying error.
    pub dl_fetch_failed_body_fmt: &'static str,
    pub dl_no_assets: &'static str,
    pub dl_confirm_title: &'static str,
    /// Format with `{name}` `{size}` `{kind}` `{release}`.
    pub dl_confirm_body_fmt: &'static str,
    pub dl_filter_label: &'static str,
    pub dl_filter_hint: &'static str,
    pub dl_category_title: &'static str,
    pub dl_assets_title: &'static str,
    pub dl_no_matches: &'static str,
    pub dl_progress_connecting: &'static str,
    pub dl_progress_title_prefix: &'static str,
    pub dl_done_title: &'static str,
    pub dl_done_body: &'static str,
    pub dl_error_title: &'static str,
    /// Format with `{}` for error.
    pub dl_error_body_fmt: &'static str,

    pub import_title: &'static str,
    pub import_body: &'static str,
    pub import_path_label: &'static str,
    pub import_empty_path: &'static str,
    pub browse_title: &'static str,
    pub browse_hint: &'static str,
    pub browse_dir_empty: &'static str,
    pub browse_select: &'static str,
    pub browse_go_up: &'static str,

    pub net_title: &'static str,
    pub net_summary_header: &'static str,
    pub net_token_label: &'static str,
    pub net_hosts_label: &'static str,
    pub net_hosts_source_label: &'static str,
    pub net_mirror_label: &'static str,
    pub net_lang_label: &'static str,
    pub net_set: &'static str,
    pub net_none: &'static str,
    pub net_enabled: &'static str,
    pub net_disabled: &'static str,
    /// Format with `{}` for entry count.
    pub net_hosts_entries_fmt: &'static str,
    pub net_action_set_token: &'static str,
    pub net_action_clear_token: &'static str,
    pub net_action_set_hosts_url: &'static str,
    pub net_action_toggle_hosts_on: &'static str,
    pub net_action_toggle_hosts_off: &'static str,
    pub net_action_pull_hosts: &'static str,
    pub net_action_add_mirror: &'static str,
    pub net_action_list_mirrors: &'static str,
    pub net_action_clear_cache: &'static str,
    pub net_action_set_silence: &'static str,
    pub net_silence_label: &'static str,

    pub silence_title: &'static str,
    pub silence_body: &'static str,

    pub token_title: &'static str,
    pub token_body: &'static str,

    pub hosts_src_title: &'static str,
    pub hosts_src_body: &'static str,

    pub mirror_add_title: &'static str,
    pub mirror_add_body: &'static str,
    pub mirror_pattern_label: &'static str,
    pub mirror_replacement_label: &'static str,
    pub mirror_pattern_empty: &'static str,
    pub mirror_list_title: &'static str,
    pub mirror_list_empty: &'static str,
    pub mirror_list_hint: &'static str,

    pub lang_title: &'static str,
    pub lang_body: &'static str,

    pub settings_title: &'static str,
    pub settings_network: &'static str,
    pub settings_translation: &'static str,
    pub settings_denoiser: &'static str,
    pub settings_language: &'static str,
    pub settings_manage_models: &'static str,
    pub aux_models_title: &'static str,
    pub aux_models_hint: &'static str,
    pub aux_models_none: &'static str,
    pub aux_kind_vad: &'static str,
    pub aux_kind_punctuation: &'static str,
    pub aux_kind_denoiser: &'static str,
    pub aux_kind_speaker: &'static str,
    pub aux_kind_translation: &'static str,
    pub confirm_delete_aux_body_fmt: &'static str,

    pub denoiser_title: &'static str,
    pub denoiser_model_label: &'static str,
    pub denoiser_voice_label: &'static str,
    pub denoiser_voice_hint: &'static str,
    pub denoiser_browse: &'static str,
    pub denoiser_save_reload: &'static str,
    pub denoiser_disable: &'static str,
    pub denoiser_select_audio_title: &'static str,
    pub denoiser_select_audio_body: &'static str,
    pub denoiser_select: &'static str,
    pub denoiser_wav_error: &'static str,
    /// Format with `{}` for the import error.
    pub voice_import_failed_fmt: &'static str,
    pub denoiser_ok: &'static str,
    pub denoiser_cancel: &'static str,
    pub denoiser_disabled: &'static str,
}

pub const EN: Strings = Strings {
    title_models: "Models",
    title_log: "Log",
    title_status: "Status",
    title_shortcuts: "Shortcuts",
    model_panel_hint: "Enter: select active   x: delete   d: download   i: import",
    log_panel_hint: "Live server log (drag in terminal to select/copy visible text)",

    help_kw_models: "Models:  ",
    help_kw_server: "Server:  ",
    help_kw_download: "download",
    help_kw_import: "import",
    help_kw_select: "select",
    help_kw_delete: "delete",
    help_kw_run_stop: "run/stop",
    help_kw_clear_cache: "clear cache",
    help_kw_help: "help",
    help_kw_quit: "quit",
    help_kw_settings: "settings",

    btn_quit: "Quit",
    btn_cancel: "Cancel",
    btn_ok: "OK",
    btn_delete: "Delete",
    btn_save: "Save",
    btn_add: "Add",
    btn_close: "Close",
    btn_refresh: "Refresh",
    btn_clear_all: "Clear All",
    btn_download: "Download",

    confirm_quit_title: "Confirm Quit",
    confirm_quit_body:
        "The WebSocket server is running.\nQuitting will stop it.\n\nQuit anyway?",

    confirm_delete_title: "Confirm Delete",
    confirm_delete_body_fmt: "Delete model #{}? This removes its files from disk.",

    cache_title: "Cache",
    cache_cleared: "Release-list cache cleared.",
    cache_failed_fmt: "Failed to clear cache: {}",

    trset_title: "Translation Settings",
    trset_mode_label: "Mode",
    trset_target_label: "Target language",
    trset_triggers_label: "Trigger phrases (comma-separated)",
    trset_strip_label: "Strip trigger & hint from input",
    trset_saved: "Translation settings saved.",

    status_ip: "IP",
    status_models_dir: "Models dir ",
    status_discovered: "Discovered ",
    status_active_asr: "Active ASR ",
    status_vad: "VAD        ",
    status_punctuation: "Punctuation",
    status_websocket: "WebSocket  ",
    status_running: "running",
    status_stopped: "stopped",
    status_mode_streaming: "streaming",
    status_mode_offline: "offline",
    status_models_count_fmt: "{} ASR model dir(s)",
    status_none: "(none)",
    status_unavailable: "(unavailable)",
    status_no_models_hint:
        "No models yet. Press [d] to download from GitHub Releases, or [i] to import a local archive/directory.",
    status_no_active_hint:
        "No active ASR model. Highlight a model in the left panel and press [s] (or Enter).",

    list_mode_stream: "stream",
    list_mode_offline: "offline",

    help_title: "Help - sherpa-onnx.server",
    help_section_getting_started: "Getting started\n",
    help_getting_started_1_a: "  1. Press ",
    help_getting_started_1_b: " to download a recommended model from GitHub Releases\n     (or ",
    help_getting_started_1_c: " to import a local .tar.bz2 / .tar.gz / .zip / directory).\n",
    help_getting_started_2_a: "  2. Highlight a model in the left panel and press ",
    help_getting_started_2_b: " (or Enter) to make it active.\n",
    help_getting_started_3_a: "  3. Press ",
    help_getting_started_3_b: " to start the WebSocket server on the configured host/port.\n",
    help_section_keys: "Keys\n",
    help_key_d: "open the download browser (with category + filter)",
    help_key_i: "import a local model archive or directory",
    help_key_s: "select the highlighted model as active",
    help_key_enter: "same as s on the model list",
    help_key_x: "delete the highlighted model (confirms first)",
    help_key_r: "start or stop the WebSocket server",
    help_key_o: "open the Settings menu",
    help_key_c: "clear the cached GitHub release list",
    help_key_q: "quit",
    help_key_help: "show this help (also F1)",
    help_section_files: "Files on disk\n",
    help_files_default: "  Default store dir: ~/.sherpa-onnx.server/\n",
    help_files_models: "  models/                 each subdir is one ASR model\n",
    help_files_punct: "  models/punctuation/     punctuation .onnx files\n",
    help_files_config: "  models/config.json      active model, mirrors, hosts, etc.\n",
    help_files_cache: "  models/release-assets-cache.json  cached GitHub release list\n",
    help_section_network: "Network (when GitHub is slow / unreachable)\n",
    help_network_press_a: "  Press ",
    help_network_press_b: " then:\n",
    help_network_token: "    Set GitHub Token         - if you hit API rate limits\n",
    help_network_mirror: "    Add Mirror Rule          - rewrite URLs, e.g.\n",
    help_network_mirror_example:
        "        Pattern:     ^https://github\\.com/\n        Replacement: https://gh-proxy.example.com/\n",
    help_network_hosts_a: "    Set Hosts Source URL + Toggle Hosts + Pull Hosts from Source\n",
    help_network_hosts_b: "      to map raw.githubusercontent.com, etc.\n",
    help_section_models: "Supported models\n",
    help_models_list_1: "  sense-voice / paraformer / transducer / whisper / nemo-ctc /\n",
    help_models_list_2: "  zipformer-ctc / online (streaming) variants.\n",
    help_models_aux_1: "  Auxiliary files (silero VAD, punctuation .onnx) are auto-detected\n",
    help_models_aux_2: "  when present in the model directory.\n",

    dl_loading: "Fetching release list from GitHub...\n(this may take a few seconds)",
    dl_title: "Download Model",
    dl_fetch_failed_title: "Download error",
    dl_fetch_failed_body_fmt: "Failed to fetch release list:\n\n{}\n\nTips:\n  - Press n to set a GitHub token or add a mirror rule.\n  - Press c to clear the cached release list and try again.",
    dl_no_assets: "No downloadable assets found.\n\nCheck Network settings (n) or refresh the release list.",
    dl_confirm_title: "Confirm Download",
    dl_confirm_body_fmt: "Download and import:\n\n  {name}\n\nSize: {size}\nKind: {kind}\nRelease: {release}",
    dl_filter_label: "Filter: ",
    dl_filter_hint: "Tip: filter supports plain text and regex (e.g. rk\\d+). Press Tab to move focus.",
    dl_category_title: "Category (Up/Down to switch)",
    dl_assets_title: "Assets (Enter to download)",
    dl_no_matches: "(no matches in this category)",
    dl_progress_connecting: "Connecting...",
    dl_progress_title_prefix: "Downloading ",
    dl_done_title: "Done",
    dl_done_body:
        "Download and import succeeded.\nIf this is your first model, press s to make it active, then r to start the server.",
    dl_error_title: "Download error",
    dl_error_body_fmt:
        "Download failed:\n{}\n\nTry: n -> Add Mirror Rule, or n -> Set GitHub Token.",

    import_title: "Import Model",
    import_body:
        "Import a local model into the store.\n\nAccepted inputs:\n  - archive: .tar.bz2 / .tar.gz / .tgz / .zip\n  - a model directory (with model.onnx + tokens.txt,\n    or encoder/decoder/joiner, etc.)\n  - a single .onnx VAD or punctuation file",
    import_path_label: "Path:",
    import_empty_path: "Path cannot be empty.",
    browse_title: "Browse Files",
    browse_hint: "Enter to navigate, Tab to switch to path input",
    browse_dir_empty: "(empty directory)",
    browse_select: "Select",
    browse_go_up: ".. (go up)",

    net_title: "Network & Mirror Settings",
    net_summary_header: "Current settings:\n",
    net_token_label: "  GitHub token : ",
    net_hosts_label: "  Hosts mapping: ",
    net_hosts_source_label: "  Hosts source : ",
    net_mirror_label: "  Mirror rules : ",
    net_lang_label: "  Language     : ",
    net_set: "set",
    net_none: "(none)",
    net_enabled: "enabled",
    net_disabled: "disabled",
    net_hosts_entries_fmt: " ({} entries)",
    net_action_set_token: "Set GitHub Token",
    net_action_clear_token: "Clear GitHub Token",
    net_action_set_hosts_url: "Set Hosts Source URL",
    net_action_toggle_hosts_on: "Toggle Hosts (currently ON; will disable)",
    net_action_toggle_hosts_off: "Toggle Hosts (currently OFF; will enable)",
    net_action_pull_hosts: "Pull Hosts from Source URL",
    net_action_add_mirror: "Add Mirror Rule (regex -> replacement)",
    net_action_list_mirrors: "List / Clear Mirror Rules",
    net_action_clear_cache: "Clear Release-list Cache",
    net_action_set_silence: "Set Silence Threshold (ms)",
    net_silence_label: "Silence (ms) ",

    silence_title: "Silence Threshold",
    silence_body: "Silence before speech-end boundary (500–5000 ms).\nDefault: 3000 ms. Applied to VAD min_silence_duration.",


    token_title: "GitHub Token",
    token_body:
        "Paste a personal access token to raise the GitHub API rate limit.\nStored locally in models/config.json. Leave blank to clear.",

    hosts_src_title: "Hosts Source URL",
    hosts_src_body:
        "URL to a hosts-style mapping file (one IP + hostname per line).\nThe Pull action below fetches this URL and replaces the in-memory map.",

    mirror_add_title: "Add Mirror Rule",
    mirror_add_body:
        "Rewrite outbound URLs with a regex.\n\nExample:\n  Pattern:     ^https://github\\.com/\n  Replacement: https://gh-proxy.example.com/",
    mirror_pattern_label: "Pattern (regex):",
    mirror_replacement_label: "Replacement:",
    mirror_pattern_empty: "Pattern cannot be empty.",
    mirror_list_title: "Mirror Rules",
    mirror_list_empty: "(no mirror rules configured)",
    mirror_list_hint: "Enter: toggle selected rule on/off",

    lang_title: "Language",
    lang_body: "Choose the interface language.",

    settings_title: "Settings",
    settings_network: "Network, mirrors and silence",
    settings_translation: "Translation",
    settings_denoiser: "Denoiser",
    settings_language: "Language",
    settings_manage_models: "Manage installed models",
    aux_models_title: "Installed Non-ASR Models",
    aux_models_hint: "Enter: delete selected   Esc: close",
    aux_models_none: "No non-ASR models installed.",
    aux_kind_vad: "VAD",
    aux_kind_punctuation: "Punctuation",
    aux_kind_denoiser: "Denoiser",
    aux_kind_speaker: "Speaker",
    aux_kind_translation: "Translation",
    confirm_delete_aux_body_fmt: "Delete \"{}\"? This removes its files from disk.",

    denoiser_title: "Denoiser Settings",
    denoiser_model_label: "Installed denoiser model:",
    denoiser_voice_label: "VoxLock reference WAV:",
    denoiser_voice_hint: "Optional; must be 16 kHz, 16-bit, and at least 3 seconds.",
    denoiser_browse: "Browse WAV",
    denoiser_save_reload: "Save and reload",
    denoiser_disable: "Disable",
    denoiser_select_audio_title: "Select Reference Audio",
    denoiser_select_audio_body: "Select a VoxLock reference WAV (at least 3 seconds):",
    denoiser_select: "Select",
    denoiser_wav_error: "Please select a .wav file.",
    voice_import_failed_fmt: "Failed to import voice sample: {}",
    denoiser_ok: "OK",
    denoiser_cancel: "Cancel",
    denoiser_disabled: "(disabled)",
};

pub const ZH_CN: Strings = Strings {
    title_models: "模型",
    title_log: "日志",
    title_status: "状态",
    title_shortcuts: "快捷键",
    model_panel_hint: "Enter: 设为当前   x: 删除   d: 下载   i: 导入",
    log_panel_hint: "实时服务日志（在终端中拖选可复制可见文本）",

    help_kw_models: "模型:    ",
    help_kw_server: "服务:    ",
    help_kw_download: "下载",
    help_kw_import: "导入",
    help_kw_select: "选用",
    help_kw_delete: "删除",
    help_kw_run_stop: "启动/停止",
    help_kw_clear_cache: "清缓存",
    help_kw_help: "帮助",
    help_kw_quit: "退出",
    help_kw_settings: "设置",

    btn_quit: "退出",
    btn_cancel: "取消",
    btn_ok: "确定",
    btn_delete: "删除",
    btn_save: "保存",
    btn_add: "添加",
    btn_close: "关闭",
    btn_refresh: "刷新",
    btn_clear_all: "全部清空",
    btn_download: "下载",

    confirm_quit_title: "确认退出",
    confirm_quit_body: "WebSocket 服务正在运行。\n退出将停止该服务。\n\n仍要退出吗？",

    confirm_delete_title: "确认删除",
    confirm_delete_body_fmt: "确定要删除第 {} 个模型？这将从磁盘移除其文件。",

    cache_title: "缓存",
    cache_cleared: "已清空发行包列表缓存。",
    cache_failed_fmt: "清空缓存失败：{}",

    trset_title: "翻译设置",
    trset_mode_label: "模式",
    trset_target_label: "目标语言",
    trset_triggers_label: "触发词（逗号分隔）",
    trset_strip_label: "剥离触发词和语言提示",
    trset_saved: "翻译设置已保存。",

    status_ip: "IP         ",
    status_models_dir: "模型目录   ",
    status_discovered: "已识别     ",
    status_active_asr: "当前 ASR   ",
    status_vad: "VAD        ",
    status_punctuation: "标点模型   ",
    status_websocket: "WebSocket  ",
    status_running: "运行中",
    status_stopped: "已停止",
    status_mode_streaming: "流式",
    status_mode_offline: "离线",
    status_models_count_fmt: "{} 个 ASR 模型目录",
    status_none: "（无）",
    status_unavailable: "（不可用）",
    status_no_models_hint:
        "尚未导入模型。按 [d] 从 GitHub Releases 下载，或按 [i] 导入本地压缩包/目录。",
    status_no_active_hint: "未设置当前 ASR 模型。请在左侧高亮一个模型后按 [s]（或 Enter）。",

    list_mode_stream: "流式",
    list_mode_offline: "离线",

    help_title: "帮助 - sherpa-onnx.server",
    help_section_getting_started: "快速上手\n",
    help_getting_started_1_a: "  1. 按 ",
    help_getting_started_1_b: " 从 GitHub Releases 下载推荐模型\n     （或按 ",
    help_getting_started_1_c: " 导入本地 .tar.bz2 / .tar.gz / .zip / 目录）。\n",
    help_getting_started_2_a: "  2. 在左侧高亮一个模型后按 ",
    help_getting_started_2_b: "（或 Enter）将其设为当前。\n",
    help_getting_started_3_a: "  3. 按 ",
    help_getting_started_3_b: " 在配置的 host/port 上启动 WebSocket 服务。\n",
    help_section_keys: "按键\n",
    help_key_d: "打开下载浏览器（含分类与过滤）",
    help_key_i: "导入本地模型压缩包或目录",
    help_key_s: "将高亮模型设为当前",
    help_key_enter: "在模型列表上等同于 s",
    help_key_x: "删除高亮模型（会先确认）",
    help_key_r: "启动或停止 WebSocket 服务",
    help_key_o: "打开设置菜单",
    help_key_c: "清空已缓存的 GitHub 发行列表",
    help_key_q: "退出",
    help_key_help: "显示本帮助（F1 同样可用）",
    help_section_files: "磁盘文件\n",
    help_files_default: "  默认存储目录：~/.sherpa-onnx.server/\n",
    help_files_models: "  models/                 每个子目录为一个 ASR 模型\n",
    help_files_punct: "  models/punctuation/     标点模型 .onnx 文件\n",
    help_files_config: "  models/config.json      当前模型、镜像、hosts 等设置\n",
    help_files_cache: "  models/release-assets-cache.json  GitHub 发行列表缓存\n",
    help_section_network: "网络（GitHub 缓慢或不可达时）\n",
    help_network_press_a: "  按 ",
    help_network_press_b: " 后：\n",
    help_network_token: "    设置 GitHub Token         - 解决 API 限流\n",
    help_network_mirror: "    添加镜像规则             - 重写 URL，例如\n",
    help_network_mirror_example:
        "        Pattern:     ^https://github\\.com/\n        Replacement: https://gh-proxy.example.com/\n",
    help_network_hosts_a: "    设置 Hosts 源 URL + 切换 Hosts 启停 + 从源拉取 Hosts\n",
    help_network_hosts_b: "      用以映射 raw.githubusercontent.com 等域名。\n",
    help_section_models: "支持的模型\n",
    help_models_list_1: "  sense-voice / paraformer / transducer / whisper / nemo-ctc /\n",
    help_models_list_2: "  zipformer-ctc / online（流式）变体。\n",
    help_models_aux_1: "  辅助文件（silero VAD、标点 .onnx）会在模型目录中\n",
    help_models_aux_2: "  自动识别。\n",

    dl_loading: "正在从 GitHub 获取发行列表...\n（可能需要数秒）",
    dl_title: "下载模型",
    dl_fetch_failed_title: "下载错误",
    dl_fetch_failed_body_fmt:
        "获取发行列表失败：\n\n{}\n\n提示：\n  - 按 n 设置 GitHub Token 或添加镜像规则。\n  - 按 c 清空缓存后重试。",
    dl_no_assets: "未找到可下载的资源。\n\n请检查网络设置（n）或刷新发行列表。",
    dl_confirm_title: "确认下载",
    dl_confirm_body_fmt:
        "下载并导入：\n\n  {name}\n\n大小：{size}\n类型：{kind}\n发行版：{release}",
    dl_filter_label: "过滤：",
    dl_filter_hint: "提示：过滤支持纯文本与正则（如 rk\\d+）。按 Tab 切换焦点。",
    dl_category_title: "分类（上下键切换）",
    dl_assets_title: "资源（按 Enter 下载）",
    dl_no_matches: "（该分类下无匹配项）",
    dl_progress_connecting: "正在连接...",
    dl_progress_title_prefix: "正在下载 ",
    dl_done_title: "完成",
    dl_done_body: "下载并导入成功。\n若这是首个模型，请按 s 将其设为当前，然后按 r 启动服务。",
    dl_error_title: "下载错误",
    dl_error_body_fmt: "下载失败：\n{}\n\n请尝试：n -> 添加镜像规则，或 n -> 设置 GitHub Token。",

    import_title: "导入模型",
    import_body:
        "将本地模型导入到存储目录。\n\n接受的输入：\n  - 压缩包：.tar.bz2 / .tar.gz / .tgz / .zip\n  - 模型目录（含 model.onnx + tokens.txt，\n    或 encoder/decoder/joiner 等）\n  - 单个 .onnx VAD 或标点文件",
    import_path_label: "路径：",
    import_empty_path: "路径不能为空。",
    browse_title: "浏览文件",
    browse_hint: "Enter 导航，Tab 切换到路径输入框",
    browse_dir_empty: "（空目录）",
    browse_select: "选择",
    browse_go_up: ".. （上级目录）",

    net_title: "网络与镜像设置",
    net_summary_header: "当前设置：\n",
    net_token_label: "  GitHub Token： ",
    net_hosts_label: "  Hosts 映射：   ",
    net_hosts_source_label: "  Hosts 源：     ",
    net_mirror_label: "  镜像规则：     ",
    net_lang_label: "  界面语言：     ",
    net_set: "已设置",
    net_none: "（无）",
    net_enabled: "启用",
    net_disabled: "禁用",
    net_hosts_entries_fmt: "（{} 条）",
    net_action_set_token: "设置 GitHub Token",
    net_action_clear_token: "清除 GitHub Token",
    net_action_set_hosts_url: "设置 Hosts 源 URL",
    net_action_toggle_hosts_on: "切换 Hosts（当前为开，将关闭）",
    net_action_toggle_hosts_off: "切换 Hosts（当前为关，将开启）",
    net_action_pull_hosts: "从源 URL 拉取 Hosts",
    net_action_add_mirror: "添加镜像规则（正则 -> 替换）",
    net_action_list_mirrors: "查看 / 清空镜像规则",
    net_action_clear_cache: "清空发行列表缓存",
    net_action_set_silence: "设置停顿阈值 (ms)",
    net_silence_label: "停顿阈值 (ms)",

    silence_title: "停顿阈值",
    silence_body: "语音停顿时长超过此值后判定为说话结束（500–5000 ms）。\n默认: 3000 ms。应用于 VAD min_silence_duration。",

    token_title: "GitHub Token",
    token_body:
        "粘贴 Personal Access Token 以提高 GitHub API 速率限制。\n仅存储在本地 models/config.json。留空则清除。",

    hosts_src_title: "Hosts 源 URL",
    hosts_src_body:
        "指向一个 hosts 格式映射文件的 URL（每行一对 IP + 域名）。\n下面的「拉取」会从该 URL 获取并替换内存中的映射。",

    mirror_add_title: "添加镜像规则",
    mirror_add_body:
        "使用正则改写对外的 URL。\n\n示例：\n  Pattern:     ^https://github\\.com/\n  Replacement: https://gh-proxy.example.com/",
    mirror_pattern_label: "Pattern（正则）：",
    mirror_replacement_label: "Replacement：",
    mirror_pattern_empty: "Pattern 不能为空。",
    mirror_list_title: "镜像规则",
    mirror_list_empty: "（尚无镜像规则）",
    mirror_list_hint: "Enter：启用/禁用所选规则",

    lang_title: "界面语言",
    lang_body: "请选择界面语言。",

    settings_title: "设置",
    settings_network: "网络、镜像与停顿",
    settings_translation: "翻译",
    settings_denoiser: "降噪",
    settings_language: "语言",
    settings_manage_models: "管理已安装模型",
    aux_models_title: "已安装的非 ASR 模型",
    aux_models_hint: "Enter：删除所选   Esc：关闭",
    aux_models_none: "没有已安装的非 ASR 模型。",
    aux_kind_vad: "VAD",
    aux_kind_punctuation: "标点",
    aux_kind_denoiser: "降噪",
    aux_kind_speaker: "声纹",
    aux_kind_translation: "翻译",
    confirm_delete_aux_body_fmt: "确定删除“{}”？这将从磁盘移除其文件。",

    denoiser_title: "降噪设置",
    denoiser_model_label: "已安装的降噪模型：",
    denoiser_voice_label: "VoxLock 参考音频：",
    denoiser_voice_hint: "可选，必须是 16 kHz、16-bit 且至少 3 秒。",
    denoiser_browse: "选择 WAV",
    denoiser_save_reload: "保存并重载",
    denoiser_disable: "禁用",
    denoiser_select_audio_title: "选择参考音频",
    denoiser_select_audio_body: "选择 VoxLock 参考 WAV（至少 3 秒）：",
    denoiser_select: "选择",
    denoiser_wav_error: "请选择 .wav 文件。",
    voice_import_failed_fmt: "导入参考音频失败：{}",
    denoiser_ok: "确定",
    denoiser_cancel: "取消",
    denoiser_disabled: "（已禁用）",
};

pub const ZH_TW: Strings = Strings {
    title_models: "模型",
    title_log: "日誌",
    title_status: "狀態",
    title_shortcuts: "快捷鍵",
    model_panel_hint: "Enter：設為當前   x：刪除   d：下載   i：匯入",
    log_panel_hint: "即時服務日誌（在終端機中拖選可複製可見文字）",

    help_kw_models: "模型:    ",
    help_kw_server: "服務:    ",
    help_kw_download: "下載",
    help_kw_import: "匯入",
    help_kw_select: "選用",
    help_kw_delete: "刪除",
    help_kw_run_stop: "啟動/停止",
    help_kw_clear_cache: "清快取",
    help_kw_help: "說明",
    help_kw_quit: "離開",
    help_kw_settings: "設定",

    btn_quit: "離開",
    btn_cancel: "取消",
    btn_ok: "確定",
    btn_delete: "刪除",
    btn_save: "儲存",
    btn_add: "新增",
    btn_close: "關閉",
    btn_refresh: "重新整理",
    btn_clear_all: "全部清除",
    btn_download: "下載",

    confirm_quit_title: "確認離開",
    confirm_quit_body: "WebSocket 服務正在執行。\n離開將停止該服務。\n\n仍要離開嗎？",

    confirm_delete_title: "確認刪除",
    confirm_delete_body_fmt: "確定要刪除第 {} 個模型？這將從磁碟移除其檔案。",

    cache_title: "快取",
    cache_cleared: "已清空發行套件清單快取。",
    cache_failed_fmt: "清空快取失敗：{}",

    trset_title: "翻譯設定",
    trset_mode_label: "模式",
    trset_target_label: "目標語言",
    trset_triggers_label: "觸發詞（逗號分隔）",
    trset_strip_label: "剝離觸發詞和語言提示",
    trset_saved: "翻譯設定已儲存。",

    status_ip: "IP         ",
    status_models_dir: "模型目錄   ",
    status_discovered: "已辨識     ",
    status_active_asr: "當前 ASR   ",
    status_vad: "VAD        ",
    status_punctuation: "標點模型   ",
    status_websocket: "WebSocket  ",
    status_running: "執行中",
    status_stopped: "已停止",
    status_mode_streaming: "串流",
    status_mode_offline: "離線",
    status_models_count_fmt: "{} 個 ASR 模型目錄",
    status_none: "（無）",
    status_unavailable: "（不可用）",
    status_no_models_hint:
        "尚未匯入模型。按 [d] 從 GitHub Releases 下載，或按 [i] 匯入本機壓縮檔/目錄。",
    status_no_active_hint: "未設定當前 ASR 模型。請在左側標記一個模型後按 [s]（或 Enter）。",

    list_mode_stream: "串流",
    list_mode_offline: "離線",

    help_title: "說明 - sherpa-onnx.server",
    help_section_getting_started: "快速入門\n",
    help_getting_started_1_a: "  1. 按 ",
    help_getting_started_1_b: " 從 GitHub Releases 下載推薦模型\n     （或按 ",
    help_getting_started_1_c: " 匯入本機 .tar.bz2 / .tar.gz / .zip / 目錄）。\n",
    help_getting_started_2_a: "  2. 在左側標記一個模型後按 ",
    help_getting_started_2_b: "（或 Enter）將其設為當前。\n",
    help_getting_started_3_a: "  3. 按 ",
    help_getting_started_3_b: " 在已設定的 host/port 上啟動 WebSocket 服務。\n",
    help_section_keys: "按鍵\n",
    help_key_d: "開啟下載瀏覽器（含分類與篩選）",
    help_key_i: "匯入本機模型壓縮檔或目錄",
    help_key_s: "將標記模型設為當前",
    help_key_enter: "在模型清單上等同於 s",
    help_key_x: "刪除標記模型（會先確認）",
    help_key_r: "啟動或停止 WebSocket 服務",
    help_key_o: "開啟設定選單",
    help_key_c: "清空已快取的 GitHub 發行清單",
    help_key_q: "離開",
    help_key_help: "顯示本說明（F1 同樣可用）",
    help_section_files: "磁碟檔案\n",
    help_files_default: "  預設儲存目錄：~/.sherpa-onnx.server/\n",
    help_files_models: "  models/                 每個子目錄為一個 ASR 模型\n",
    help_files_punct: "  models/punctuation/     標點模型 .onnx 檔\n",
    help_files_config: "  models/config.json      當前模型、鏡像、hosts 等設定\n",
    help_files_cache: "  models/release-assets-cache.json  GitHub 發行清單快取\n",
    help_section_network: "網路（GitHub 緩慢或無法連線時）\n",
    help_network_press_a: "  按 ",
    help_network_press_b: " 後：\n",
    help_network_token: "    設定 GitHub Token         - 解決 API 限流\n",
    help_network_mirror: "    新增鏡像規則             - 改寫 URL，例如\n",
    help_network_mirror_example:
        "        Pattern:     ^https://github\\.com/\n        Replacement: https://gh-proxy.example.com/\n",
    help_network_hosts_a: "    設定 Hosts 來源 URL + 切換 Hosts 啟停 + 從來源抓取 Hosts\n",
    help_network_hosts_b: "      用以對應 raw.githubusercontent.com 等網域。\n",
    help_section_models: "支援的模型\n",
    help_models_list_1: "  sense-voice / paraformer / transducer / whisper / nemo-ctc /\n",
    help_models_list_2: "  zipformer-ctc / online（串流）變體。\n",
    help_models_aux_1: "  輔助檔（silero VAD、標點 .onnx）會在模型目錄中\n",
    help_models_aux_2: "  自動辨識。\n",

    dl_loading: "正在從 GitHub 取得發行清單...\n（可能需要數秒）",
    dl_title: "下載模型",
    dl_fetch_failed_title: "下載錯誤",
    dl_fetch_failed_body_fmt:
        "取得發行清單失敗：\n\n{}\n\n提示：\n  - 按 n 設定 GitHub Token 或新增鏡像規則。\n  - 按 c 清空快取後重試。",
    dl_no_assets: "找不到可下載的資源。\n\n請檢查網路設定（n）或重新整理發行清單。",
    dl_confirm_title: "確認下載",
    dl_confirm_body_fmt:
        "下載並匯入：\n\n  {name}\n\n大小：{size}\n類型：{kind}\n發行版：{release}",
    dl_filter_label: "篩選：",
    dl_filter_hint: "提示：篩選支援純文字與正則（如 rk\\d+）。按 Tab 切換焦點。",
    dl_category_title: "分類（上下鍵切換）",
    dl_assets_title: "資源（按 Enter 下載）",
    dl_no_matches: "（此分類下無相符項）",
    dl_progress_connecting: "正在連線...",
    dl_progress_title_prefix: "正在下載 ",
    dl_done_title: "完成",
    dl_done_body: "下載並匯入成功。\n若這是第一個模型，請按 s 將其設為當前，再按 r 啟動服務。",
    dl_error_title: "下載錯誤",
    dl_error_body_fmt: "下載失敗：\n{}\n\n請嘗試：n -> 新增鏡像規則，或 n -> 設定 GitHub Token。",

    import_title: "匯入模型",
    import_body:
        "將本機模型匯入儲存目錄。\n\n接受的輸入：\n  - 壓縮檔：.tar.bz2 / .tar.gz / .tgz / .zip\n  - 模型目錄（含 model.onnx + tokens.txt，\n    或 encoder/decoder/joiner 等）\n  - 單一 .onnx VAD 或標點檔",
    import_path_label: "路徑：",
    import_empty_path: "路徑不能為空。",
    browse_title: "瀏覽檔案",
    browse_hint: "Enter 導覽，Tab 切換到路徑輸入框",
    browse_dir_empty: "（空目錄）",
    browse_select: "選擇",
    browse_go_up: ".. （上層目錄）",

    net_title: "網路與鏡像設定",
    net_summary_header: "目前設定：\n",
    net_token_label: "  GitHub Token： ",
    net_hosts_label: "  Hosts 對應：   ",
    net_hosts_source_label: "  Hosts 來源：   ",
    net_mirror_label: "  鏡像規則：     ",
    net_lang_label: "  介面語言：     ",
    net_set: "已設定",
    net_none: "（無）",
    net_enabled: "啟用",
    net_disabled: "停用",
    net_hosts_entries_fmt: "（{} 筆）",
    net_action_set_token: "設定 GitHub Token",
    net_action_clear_token: "清除 GitHub Token",
    net_action_set_hosts_url: "設定 Hosts 來源 URL",
    net_action_toggle_hosts_on: "切換 Hosts（目前為開，將關閉）",
    net_action_toggle_hosts_off: "切換 Hosts（目前為關，將開啟）",
    net_action_pull_hosts: "從來源 URL 抓取 Hosts",
    net_action_add_mirror: "新增鏡像規則（正則 -> 替換）",
    net_action_list_mirrors: "檢視 / 清空鏡像規則",
    net_action_clear_cache: "清空發行清單快取",
    net_action_set_silence: "設定停頓閾值 (ms)",
    net_silence_label: "停頓閾值 (ms)",

    silence_title: "停頓閾值",
    silence_body: "語音停頓時長超過此值後判定為說話結束（500–5000 ms）。\n預設: 3000 ms。應用於 VAD min_silence_duration。",

    token_title: "GitHub Token",
    token_body:
        "貼上 Personal Access Token 以提高 GitHub API 速率限制。\n僅儲存於本機 models/config.json。留空則清除。",

    hosts_src_title: "Hosts 來源 URL",
    hosts_src_body:
        "指向 hosts 格式對應檔的 URL（每行一組 IP + 主機名）。\n下方「抓取」會從該 URL 取得並取代記憶體中的對應。",

    mirror_add_title: "新增鏡像規則",
    mirror_add_body:
        "使用正則改寫對外的 URL。\n\n範例：\n  Pattern:     ^https://github\\.com/\n  Replacement: https://gh-proxy.example.com/",
    mirror_pattern_label: "Pattern（正則）：",
    mirror_replacement_label: "Replacement：",
    mirror_pattern_empty: "Pattern 不能為空。",
    mirror_list_title: "鏡像規則",
    mirror_list_empty: "（尚無鏡像規則）",
    mirror_list_hint: "Enter：啟用/停用所選規則",

    lang_title: "介面語言",
    lang_body: "請選擇介面語言。",

    settings_title: "設定",
    settings_network: "網路、鏡像與停頓",
    settings_translation: "翻譯",
    settings_denoiser: "降噪",
    settings_language: "語言",
    settings_manage_models: "管理已安裝模型",
    aux_models_title: "已安裝的非 ASR 模型",
    aux_models_hint: "Enter：刪除所選   Esc：關閉",
    aux_models_none: "沒有已安裝的非 ASR 模型。",
    aux_kind_vad: "VAD",
    aux_kind_punctuation: "標點",
    aux_kind_denoiser: "降噪",
    aux_kind_speaker: "聲紋",
    aux_kind_translation: "翻譯",
    confirm_delete_aux_body_fmt: "確定刪除「{}」？這將從磁碟移除其檔案。",

    denoiser_title: "降噪設定",
    denoiser_model_label: "已安裝的降噪模型：",
    denoiser_voice_label: "VoxLock 參考音訊：",
    denoiser_voice_hint: "可選，必須是 16 kHz、16-bit 且至少 3 秒。",
    denoiser_browse: "選擇 WAV",
    denoiser_save_reload: "儲存並重新載入",
    denoiser_disable: "停用",
    denoiser_select_audio_title: "選擇參考音訊",
    denoiser_select_audio_body: "選擇 VoxLock 參考 WAV（至少 3 秒）：",
    denoiser_select: "選擇",
    denoiser_wav_error: "請選擇 .wav 檔案。",
    voice_import_failed_fmt: "匯入參考音訊失敗：{}",
    denoiser_ok: "確定",
    denoiser_cancel: "取消",
    denoiser_disabled: "（已停用）",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_round_trip() {
        for lang in Lang::all() {
            assert_eq!(Lang::from_code(lang.code()), Some(*lang));
        }
    }

    #[test]
    fn accepts_common_locale_aliases() {
        assert_eq!(Lang::from_code("ja_JP"), Some(Lang::Ja));
        assert_eq!(Lang::from_code("ko-KR"), Some(Lang::Ko));
        assert_eq!(Lang::from_code("ru_RU"), Some(Lang::Ru));
        assert_eq!(Lang::from_code("es-419"), Some(Lang::Es));
        assert_eq!(Lang::from_code("pt_BR"), Some(Lang::PtBr));
        assert_eq!(Lang::from_code("fr_FR"), Some(Lang::Fr));
        assert_eq!(Lang::from_code("de_DE"), Some(Lang::De));
    }
}
