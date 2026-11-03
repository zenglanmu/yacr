//! An in-memory [`FontLoader`] for hosts that ship the font package as bytes.
//!
//! The Web host fetches `fonts.json` and the bundled faces through its network
//! stack, but a host that already has them in memory (a generated bundle, a
//! test) must not grow a second, fake loader. This loader serves exactly the
//! bytes it was given and reports a missing file as a named
//! [`CadError::ResourceMissing`] instead of pretending an empty face loaded.

use std::collections::HashMap;
use std::sync::Arc;

use cad_domain::CadError;

use crate::{FontLoader, HostFuture};

/// A [`FontLoader`] backed by an in-memory catalog and file map.
pub struct MemoryFontLoader {
    catalog: Arc<[u8]>,
    /// Keyed by lowercased bare file name.
    files: HashMap<String, Arc<[u8]>>,
}

impl MemoryFontLoader {
    /// Build from an already-materialised catalog plus file bytes.
    ///
    /// File keys are normalised to their lowercased bare file name, matching
    /// the case-insensitive lookup [`FontLoader::load_font`] performs.
    pub fn new(catalog: Arc<[u8]>, files: HashMap<String, Arc<[u8]>>) -> Self {
        let files = files
            .into_iter()
            .map(|(name, bytes)| (normalize_name(&name), bytes))
            .collect();
        Self { catalog, files }
    }

    /// Build from owned catalog bytes and per-file byte vectors.
    pub fn from_bytes(
        catalog: Vec<u8>,
        files: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Self {
        let files = files
            .into_iter()
            .map(|(name, bytes)| (name, Arc::from(bytes.into_boxed_slice())))
            .collect();
        Self::new(Arc::from(catalog.into_boxed_slice()), files)
    }

    /// Number of individual font files held (the catalog is not counted).
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

/// Strip a query/fragment, take the final path segment, lowercase it.
fn normalize_name(url: &str) -> String {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/');
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

impl FontLoader for MemoryFontLoader {
    fn load_catalog(&self, _url: &str) -> HostFuture<'_, Arc<[u8]>> {
        let catalog = Arc::clone(&self.catalog);
        Box::pin(std::future::ready(Ok(catalog)))
    }

    fn load_font(&self, url: &str) -> HostFuture<'_, Arc<[u8]>> {
        let name = normalize_name(url);
        let result = match self.files.get(&name) {
            Some(bytes) => Ok(Arc::clone(bytes)),
            None => Err(CadError::ResourceMissing(format!(
                "font file not in the in-memory bundle: {name}"
            ))),
        };
        Box::pin(std::future::ready(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &[u8] = br#"[{"file":"a.shx","name":["a"],"type":"shx"}]"#;

    fn loader() -> MemoryFontLoader {
        MemoryFontLoader::new(
            Arc::from(CATALOG),
            HashMap::from([("a.shx".to_string(), Arc::from(b"font-bytes".as_slice()))]),
        )
    }

    #[test]
    fn serves_the_catalog_and_looks_files_up_case_insensitively() {
        let loader = loader();
        assert_eq!(loader.file_count(), 1);
        let catalog = crate::block_on(loader.load_catalog("https://x/fonts/fonts.json")).unwrap();
        assert_eq!(&catalog[..], CATALOG);
        // Query string, fragment and different case must not defeat the lookup.
        let bytes = crate::block_on(loader.load_font("https://x/fonts/A.SHX?v=2#frag")).unwrap();
        assert_eq!(&bytes[..], b"font-bytes");
    }

    #[test]
    fn from_bytes_normalises_keys_too() {
        let loader = MemoryFontLoader::from_bytes(
            CATALOG.to_vec(),
            [("Fonts/A.SHX".to_string(), b"bytes".to_vec())],
        );
        assert_eq!(loader.file_count(), 1);
        assert_eq!(
            &crate::block_on(loader.load_font("https://x/fonts/a.shx")).unwrap()[..],
            b"bytes"
        );
    }

    #[test]
    fn a_missing_file_is_a_named_error_never_empty_bytes() {
        let loader = loader();
        let error = crate::block_on(loader.load_font("https://x/fonts/Noto Sans.otf")).unwrap_err();
        match error {
            CadError::ResourceMissing(message) => {
                assert!(message.contains("noto sans.otf"), "{message}");
            }
            other => panic!("expected ResourceMissing, got {other:?}"),
        }
    }
}
