use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Interface,
    TypeAlias,
    Variable,
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SymbolKind::Function => write!(f, "function"),
            SymbolKind::Class => write!(f, "class"),
            SymbolKind::Method => write!(f, "method"),
            SymbolKind::Interface => write!(f, "interface"),
            SymbolKind::TypeAlias => write!(f, "type"),
            SymbolKind::Variable => write!(f, "variable"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ByteRange {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub name: String,
    pub range: ByteRange,
    pub signature: String,
    pub is_exported: bool,
    pub parent_class: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Import {
    pub names: Vec<String>,
    pub source: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ExtractedFile {
    pub file_path: PathBuf,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
    pub exports: Vec<String>,
    pub warnings: Vec<String>,
    pub mtime: std::time::SystemTime,
}
