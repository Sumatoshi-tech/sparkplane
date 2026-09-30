use std::borrow::Cow;
pub trait BlockHandler {
    /// Rewrite a raw line before it is matched or emitted — the place to
    /// strip ANSI escapes for tools that colour by default. Identity unless
    /// a handler opts in, so the rest of the pipeline sees raw bytes.
    fn normalize_line<'a>(&self, line: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(line)
    }
    fn should_skip(&mut self, line: &str) -> bool;
    fn is_block_start(&mut self, line: &str) -> bool;
    fn is_block_continuation(&mut self, line: &str, block: &[String]) -> bool;
    fn format_summary(&self, exit_code: i32, raw: &str) -> Option<String>;
}
