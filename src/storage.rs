use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use loro::LoroDoc;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Bytes(Vec<u8>);
pub type DocumentId = Uuid;

pub trait Storage {
    fn new() -> Self;
    async fn load_document(&mut self, id: &DocumentId) -> Document;
    async fn save_document(&mut self, doc: &Document);
}

#[derive(Debug, Clone)]
pub struct Document {
    pub(crate) doc: Arc<LoroDoc>,
    pub(crate) id: DocumentId,
}

impl Document {
    pub fn get_loro_doc(&self) -> &LoroDoc {
        &self.doc
    }
}

pub mod generic {
    use std::{collections::HashMap, sync::Arc};

    use loro::{ExportMode, LoroDoc};
    use uuid::Uuid;

    use super::{Document, Storage};

    #[derive(Debug, Default)]
    pub struct MemoryStorage {
        storage: HashMap<Uuid, Arc<LoroDoc>>,
    }

    impl Storage for MemoryStorage {
        fn new() -> Self {
            Self::default()
        }

        async fn load_document(&mut self, id: &super::DocumentId) -> super::Document {
            let doc = self
                .storage
                .get(id)
                .map(|x| x.clone())
                .unwrap_or_else(|| Arc::new(LoroDoc::new()));
            Document { id: *id, doc }
        }

        async fn save_document(&mut self, doc: &super::Document) {
            if let Some(old_doc) = self.storage.get(&doc.id) {
                old_doc
                    .import(&doc.doc.export(ExportMode::Snapshot).unwrap())
                    .unwrap();
            } else {
                self.storage.insert(doc.id, doc.doc.clone());
            }
        }
    }
}

pub mod server {
    use super::Bytes;
    use super::DocumentId;
    use super::Storage;
    use dioxus::{
        logger::tracing::debug,
        prelude::{server_fn::codec::Json, *},
    };
    use loro::ToJson;
    use loro::{ExportMode, LoroDoc};
    use std::{
        collections::HashMap,
        sync::{Arc, OnceLock, RwLock},
    };
    use uuid::Uuid;

    fn storage() -> &'static RwLock<HashMap<Uuid, Arc<LoroDoc>>> {
        static STORAGE: OnceLock<RwLock<HashMap<Uuid, Arc<LoroDoc>>>> = OnceLock::new();
        STORAGE.get_or_init(|| RwLock::new(HashMap::new()))
    }

    pub struct RemoteMemoryStorage {
        open_docs: HashMap<DocumentId, Arc<LoroDoc>>,
    }

    impl Storage for RemoteMemoryStorage {
        fn new() -> Self {
            Self {
                open_docs: HashMap::new(),
            }
        }

        async fn load_document(&mut self, id: &super::DocumentId) -> super::Document {
            debug!("load_document");
            let doc = if let Some(Some(Bytes(bytes))) = load(*id).await.ok() {
                debug!("load_document: loading from snapshot");
                // LoroDoc::from_snapshot(&bytes).unwrap() // currently a bug: https://discord.com/channels/1069799218614640721/1168069498918670336/1377923406179340330
                let doc = LoroDoc::new();
                doc.import(&bytes).unwrap();
                doc
            } else {
                debug!("load_document: no bytes, loading new doc");
                LoroDoc::new()
            };
            let doc = Arc::new(doc);
            debug!("load_document: {}", doc.get_deep_value().to_json_value());
            self.open_docs.insert(*id, doc.clone());
            super::Document {
                id: *id,
                doc: doc.clone(),
            }
        }

        async fn save_document(&mut self, doc: &super::Document) {
            save(doc.id, Bytes(doc.doc.export(ExportMode::Snapshot).unwrap()))
                .await
                .unwrap();
        }
    }

    #[server(input = Json, output = Json)]
    async fn load(id: super::DocumentId) -> Result<Option<Bytes>, ServerFnError> {
        Ok(storage()
            .read()
            .unwrap()
            .get(&id)
            .and_then(|doc| doc.export(ExportMode::Snapshot).ok())
            .map(|x| Bytes(x)))
    }

    #[server(input = Json, output = Json)]
    async fn save(id: super::DocumentId, bytes: Bytes) -> Result<(), ServerFnError> {
        if let Some(doc) = storage().read().unwrap().get(&id) {
            doc.import(&bytes.0).unwrap();
        } else {
            let doc = LoroDoc::from_snapshot(&bytes.0).unwrap();
            storage().write().unwrap().insert(id, doc.into());
        }
        Ok(())
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
        async fn load_document_bytes(&self, key: &DocumentId) -> Option<Vec<u8>> {
            use gloo_storage::{SessionStorage, Storage};
            let bytes: Result<Vec<u8>, _> = SessionStorage::get(key.to_string());
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

        async fn save_document(&mut self, doc: &Document) {
            use gloo_storage::{SessionStorage, Storage};
            let bytes = doc
                .doc
                .export(ExportMode::snapshot())
                .expect("export document failed");
            SessionStorage::set(&doc.id.to_string(), bytes).expect("save document failed");
        }

        async fn load_document(&mut self, id: &DocumentId) -> Document {
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
                doc: doc.into(),
            }
        }
    }
}
