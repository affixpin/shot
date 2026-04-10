pub(crate) mod shell;
pub(crate) mod file_read;
pub(crate) mod file_write;
pub(crate) mod list_files;
pub(crate) mod search_text;
pub(crate) mod send_file;
pub(crate) mod memory;

use crate::react::ToolDef;
use std::future::Future;
use std::pin::Pin;

// ── Tool trait ──────────────────────────────────────────────────────────

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn definition(&self) -> ToolDef;
    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>>;
}
