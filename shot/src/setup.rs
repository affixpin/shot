// Embedded defaults baked into the binary at compile time. `Config::load`
// extracts these to disk on first run if the target directory doesn't exist.

pub const DEFAULT_SOUL: &str = include_str!("../defaults/SOUL.md");

pub const DEFAULT_TOOLS: &[(&str, &str)] = &[
    ("file_read.toml", include_str!("../defaults/tools/file_read.toml")),
    ("file_write.toml", include_str!("../defaults/tools/file_write.toml")),
    ("file_remove.toml", include_str!("../defaults/tools/file_remove.toml")),
    ("list_files.toml", include_str!("../defaults/tools/list_files.toml")),
    ("search_text.toml", include_str!("../defaults/tools/search_text.toml")),
    ("shell.toml", include_str!("../defaults/tools/shell.toml")),
    ("memory_store.toml", include_str!("../defaults/tools/memory_store.toml")),
    ("memory_recall.toml", include_str!("../defaults/tools/memory_recall.toml")),
    ("web_search.toml", include_str!("../defaults/tools/web_search.toml")),
    ("web_read.toml", include_str!("../defaults/tools/web_read.toml")),
    ("tg_send.toml", include_str!("../defaults/tools/tg_send.toml")),
];

pub const DEFAULT_SKILLS: &[(&str, &str)] = &[
    ("project_manager.md", include_str!("../defaults/skills/project_manager.md")),
];
