use crate::semantic::types::{ByteRange, ExtractedFile};

pub trait LanguageAdapter: Send + Sync {
    fn extensions(&self) -> &[&str];

    fn extract(&self, file_path: &std::path::Path, source: &str) -> Result<ExtractedFile, String>;

    fn find_callees_in_range(
        &self,
        source: &str,
        file_path: &std::path::Path,
        range: ByteRange,
    ) -> Result<Vec<String>, String>;
}
