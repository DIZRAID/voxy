/// Все команды приложения (#[tauri::command] в lib.rs, generate_handler!).
///
/// build.rs создаёт по этому списку разрешения allow-<команда> и
/// deny-<команда> (подчёркивания становятся дефисами), и тогда Tauri
/// проверяет ACL для каждой команды: команду, которую окну не разрешили в
/// capabilities/*.json, вызвать из этого окна нельзя. Новую команду нужно
/// добавить сюда и выдать нужному окну, иначе её вызов будет отклонён
/// (тест ipc_is_scoped_per_window в lib.rs это проверяет).
///
/// Файл подключается и в build.rs (include!), поэтому здесь только const.
pub const APP_COMMANDS: &[&str] = &[
    "get_settings",
    "set_settings",
    "list_mics",
    "get_history",
    "clear_history",
    "copy_text",
    "get_model_status",
    "models_overview",
    "model_download",
    "model_cancel_download",
    "model_delete",
    "model_activate",
    "provider_save_key",
    "provider_delete_key",
    "begin_hotkey_capture",
    "cancel_hotkey_capture",
    "cancel_recording",
    "permissions_status",
    "open_permission_settings",
    "island_metrics",
    "open_url",
];
