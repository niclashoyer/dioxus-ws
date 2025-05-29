use loro::LoroDoc;

pub type DocumentId = String;

pub trait Storage {
    fn new() -> Self;
    async fn load_document(&self, id: &DocumentId) -> Document;
    async fn save_document(&self, doc: &Document);
}

#[derive(Debug)]
pub struct Document {
    pub(crate) doc: LoroDoc,
    pub(crate) id: DocumentId,
}

impl Document {
    pub fn get_loro_doc(&self) -> &LoroDoc {
        &self.doc
    }
}

pub mod web {
    use super::{Document, DocumentId, Storage};
    use loro::{ExportMode, LoroDoc};

    pub struct SessionStorage {}

    impl Default for SessionStorage {
        fn default() -> Self {
            Self {}
        }
    }

    impl SessionStorage {
        async fn load_document_bytes(&self, key: &str) -> Option<Vec<u8>> {
            use gloo_storage::{SessionStorage, Storage};
            let bytes: Result<Vec<u8>, _> = SessionStorage::get(key);
            if let Ok(bytes) = bytes {
                Some(bytes)
            } else {
                None
            }
        }
    }

    impl Storage for SessionStorage {
        fn new() -> Self {
            Self {}
        }

        async fn save_document(&self, doc: &Document) {
            use gloo_storage::{SessionStorage, Storage};
            let bytes = doc
                .doc
                .export(ExportMode::snapshot())
                .expect("export document failed");
            SessionStorage::set(doc.id.clone(), bytes).expect("save document failed");
        }

        async fn load_document(&self, id: &DocumentId) -> Document {
            #[allow(unused_variables)]
            let bytes: Option<Vec<u8>> = None;
            #[cfg(feature = "web")]
            let bytes = self.load_document_bytes(id).await;
            let doc = LoroDoc::new();
            if let Some(bytes) = bytes {
                doc.import(&bytes).expect("load document failed");
            }
            Document {
                id: id.clone(),
                doc,
            }
        }
    }
}
