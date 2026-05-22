use indexmap::IndexMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ignore::WalkBuilder;

use crate::agent::tools::MAX_GREP_RESULTS;

const MAX_INDEX_FILES: usize = 2000;
use crate::semantic::adapters::AdapterRegistry;
use crate::semantic::types::{ExtractedFile, Symbol, SymbolKind};

/// Per-file cache. `IndexMap` preserves insertion order so the
/// LRU-eviction in `ensure_file` can drop the oldest entry by
/// position. Switched from `HashMap` for audit L14 — the per-file
/// `ensure_file` path used to insert unconditionally with no cap,
/// only the bulk `ensure_all` honored `MAX_INDEX_FILES`. A long
/// session repeatedly inspecting different files in a giant
/// monorepo grew the cache without bound.
type FileCache = IndexMap<PathBuf, ExtractedFile>;

pub struct SymbolIndex {
    registry: Arc<AdapterRegistry>,
    /// Interior mutability so every public method can take `&self`.
    /// Audit M13: the tools wrapping `Arc<RwLock<SymbolIndex>>` used
    /// to grab `.write()` even for pure read paths because `&mut self`
    /// methods needed exclusive access to the cache. Moving the
    /// mutation inside a `Mutex<FileCache>` lets the outer `RwLock`
    /// stay in read mode so concurrent semantic tool invocations on
    /// the same index don't serialize.
    cache: Mutex<FileCache>,
}

impl SymbolIndex {
    pub fn new(registry: Arc<AdapterRegistry>) -> Self {
        Self {
            registry,
            cache: Mutex::new(IndexMap::new()),
        }
    }

    fn cache_lock(&self) -> std::sync::MutexGuard<'_, FileCache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Populate or refresh the cache entry for `path` and return a
    /// CLONE of the extracted file. The clone is mandatory because
    /// the returned data outlives the `Mutex<FileCache>` lock.
    /// Per-file extracts are small (dozens of symbols), so the
    /// clone cost is dwarfed by the saved write-lock contention.
    pub fn ensure_file(&self, path: &Path) -> Result<ExtractedFile, String> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        let meta = std::fs::metadata(&canonical).ok();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok());
        let size = meta.as_ref().map(|m| m.len());

        // Audit C5+M7: mtime alone is unreliable on filesystems with
        // second (or worse) granularity — a same-second write after
        // the cache fill produces an identical mtime and we serve
        // stale extracts. Combine mtime + size; size catches almost
        // every real edit (anything but a same-size in-place patch)
        // without the cost of hashing the file content.
        // Fast-path: check cache freshness under a brief lock.
        {
            let cache = self.cache_lock();
            if let Some(entry) = cache.get(&canonical) {
                let mtime_unchanged = mtime.map_or(false, |mt| mt == entry.mtime);
                let size_unchanged = size.map_or(false, |sz| sz == entry.size);
                if mtime_unchanged && size_unchanged {
                    return Ok(entry.clone());
                }
            }
        }

        // Cache miss / stale — extract outside the lock so other
        // threads can keep reading other files concurrently.
        let source = std::fs::read_to_string(&canonical).map_err(|e| format!("Read error: {e}"))?;
        // Audit L3: when picking the adapter for `.h` files, pass
        // the source content so a C++ header (with `class`,
        // `namespace`, `template`, etc.) routes to the C++ adapter
        // instead of being parsed as C.
        let adapter = self
            .registry
            .find_for_file_with_content(&canonical, Some(&source))
            .ok_or_else(|| format!("No language adapter for file: {}", canonical.display()))?;
        let mut extracted = adapter.extract(&canonical, &source)?;
        if let Some(mt) = mtime {
            extracted.mtime = mt;
        }
        if let Some(sz) = size {
            extracted.size = sz;
        }

        let mut cache = self.cache_lock();
        // Audit L14: LRU-evict the oldest entry before insert if
        // we're at the cap. Only fires when the path isn't
        // already in the cache (an in-place refresh keeps the
        // existing slot and bumps it to most-recent below).
        if !cache.contains_key(&canonical) && cache.len() >= MAX_INDEX_FILES {
            cache.shift_remove_index(0);
        }
        cache.insert(canonical.clone(), extracted.clone());
        // Move the just-touched entry to the most-recent end so it
        // survives the next eviction.
        if let Some((idx, _, _)) = cache.get_full(&canonical) {
            let last = cache.len().saturating_sub(1);
            if idx != last {
                cache.move_index(idx, last);
            }
        }
        Ok(extracted)
    }

    pub fn ensure_all(&self, root: &Path, include: Option<&str>) -> Result<usize, String> {
        let mut count = 0;
        let extensions = self.registry.all_extensions();

        let mut walker = WalkBuilder::new(root);
        walker
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .hidden(false)
            .filter_entry(move |entry| {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_str().unwrap_or("");
                    !matches!(name, "node_modules" | "target" | ".git" | "__pycache__")
                } else {
                    true
                }
            });

        for entry in walker
            .build()
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        {
            if count >= MAX_INDEX_FILES {
                break;
            }

            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if !extensions.iter().any(|e| e == &ext) {
                continue;
            }

            if let Some(pattern) = include {
                let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if let Ok(re) = regex::Regex::new(pattern) {
                    if !re.is_match(fname) {
                        continue;
                    }
                }
            }

            if let Ok(meta) = path.metadata()
                && meta.len() > 10 * 1024 * 1024
            {
                continue;
            }

            if self.ensure_file(path).is_ok() {
                count += 1;
            }
        }

        Ok(count)
    }

    pub fn find_definition(&self, name: &str) -> Result<Vec<(PathBuf, Symbol)>, String> {
        let mut results = Vec::new();

        let is_empty = self.cache_lock().is_empty();
        if is_empty {
            self.ensure_all(
                &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                None,
            )?;
        }

        let file_paths: Vec<PathBuf> = self.cache_lock().keys().cloned().collect();
        for path in file_paths {
            let Ok(entry) = self.ensure_file(&path) else {
                continue;
            };
            for sym in &entry.symbols {
                if sym.name == name {
                    results.push((entry.file_path.clone(), sym.clone()));
                }
            }
        }

        Ok(results)
    }

    pub fn find_callers(&self, name: &str, root: &Path) -> Result<Vec<String>, String> {
        let mut results = Vec::new();

        let extensions = self.registry.all_extensions();
        let re = regex::Regex::new(&format!(r"\b{}\b", regex::escape(name)))
            .map_err(|e| format!("Regex error: {e}"))?;

        let mut walker = WalkBuilder::new(root);
        walker
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .hidden(false)
            .filter_entry(|entry| {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    // Share the central skip list with find_files /
                    // glob / list_dir / grep — the previous inline
                    // `matches!` only listed 4 dirs and diverged
                    // silently from the canonical set in
                    // `agent::tools::is_skip_dir`.
                    let name = entry.file_name().to_str().unwrap_or("");
                    !crate::agent::tools::is_skip_dir(name)
                } else {
                    true
                }
            });

        for entry in walker
            .build()
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        {
            if results.len() >= MAX_GREP_RESULTS {
                break;
            }

            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if !extensions.iter().any(|e| e == &ext) {
                continue;
            }

            if let Ok(meta) = path.metadata()
                && meta.len() > 10 * 1024 * 1024
            {
                continue;
            }

            let Ok(data) = std::fs::read(path) else {
                continue;
            };

            if data.iter().take(8192).any(|&b| b == 0) {
                continue;
            }

            let content = String::from_utf8_lossy(&data);
            let path_str = path.to_string_lossy();

            let path_canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let entry = self.ensure_file(&path_canonical).ok();

            for (line_num, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    if let Some(ref entry) = entry {
                        let is_definition = entry.symbols.iter().any(|s| {
                            s.name == name
                                && s.range.start_line <= line_num + 1
                                && s.range.end_line >= line_num + 1
                        });
                        if is_definition {
                            continue;
                        }
                    }
                    results.push(format!("{}:{}: {}", path_str, line_num + 1, line.trim()));
                    if results.len() >= MAX_GREP_RESULTS {
                        break;
                    }
                }
            }
        }

        Ok(results)
    }

    pub fn find_callees(&self, file_path: &Path, name: &str) -> Result<Vec<String>, String> {
        let entry = self.ensure_file(file_path)?;

        let matches: Vec<&Symbol> = entry.symbols.iter().filter(|s| s.name == name).collect();

        if matches.is_empty() {
            return Err(format!(
                "Symbol '{}' not found in {}",
                name,
                file_path.display()
            ));
        }

        if matches.len() > 1 {
            let hints: Vec<String> = matches
                .iter()
                .map(|s| {
                    format!(
                        "  {} [{}] at line {}",
                        s.name,
                        match &s.parent_class {
                            Some(c) => format!("method of {}", c),
                            None => s.kind.to_string(),
                        },
                        s.range.start_line
                    )
                })
                .collect();
            return Err(format!(
                "Multiple symbols named '{}' found in {}:\n{}\n\nUse a more specific identifier or try get_symbol_body to inspect candidates.",
                name,
                file_path.display(),
                hints.join("\n")
            ));
        }

        let symbol = matches[0];
        let range = symbol.range;
        let source = std::fs::read_to_string(file_path).map_err(|e| format!("Read error: {e}"))?;

        if range.start_byte >= source.len() || range.end_byte > source.len() {
            return Err(
                "File modified since last parse — byte range is stale. Re-run the query."
                    .to_string(),
            );
        }

        let adapter = self
            .registry
            .find_for_file(file_path)
            .ok_or_else(|| format!("No language adapter for file: {}", file_path.display()))?;

        adapter.find_callees_in_range(&source, file_path, range)
    }

    pub fn get_symbol_body(&self, file_path: &Path, name: &str) -> Result<String, String> {
        let entry = self.ensure_file(file_path)?;

        let symbol = entry
            .symbols
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("Symbol '{}' not found in {}", name, file_path.display()))?;

        let source = std::fs::read_to_string(file_path).map_err(|e| format!("Read error: {e}"))?;

        if symbol.range.start_byte >= source.len() || symbol.range.end_byte > source.len() {
            return Err(
                "File modified since last parse — byte range is stale. Re-run the query."
                    .to_string(),
            );
        }

        let bytes = source.as_bytes();
        let body_slice = &bytes[symbol.range.start_byte..symbol.range.end_byte];
        Ok(String::from_utf8_lossy(body_slice).to_string())
    }

    pub fn list_symbols(
        &self,
        file_path: Option<&Path>,
        kind_filter: Option<SymbolKind>,
    ) -> Result<Vec<(PathBuf, Vec<Symbol>)>, String> {
        let mut result: Vec<(PathBuf, Vec<Symbol>)> = Vec::new();

        if let Some(path) = file_path {
            let entry = self.ensure_file(path)?;
            let symbols: Vec<Symbol> = entry
                .symbols
                .iter()
                .filter(|s| kind_filter.map_or(true, |k| s.kind == k))
                .cloned()
                .collect();
            result.push((entry.file_path.clone(), symbols));
        } else {
            let is_empty = self.cache_lock().is_empty();
            if is_empty {
                self.ensure_all(
                    &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                    None,
                )?;
            }
            let file_paths: Vec<PathBuf> = self.cache_lock().keys().cloned().collect();
            for path in file_paths {
                let Ok(entry) = self.ensure_file(&path) else {
                    continue;
                };
                let symbols: Vec<Symbol> = entry
                    .symbols
                    .iter()
                    .filter(|s| kind_filter.map_or(true, |k| s.kind == k))
                    .cloned()
                    .collect();
                if !symbols.is_empty() {
                    result.push((entry.file_path.clone(), symbols));
                }
            }
        }

        Ok(result)
    }
}
