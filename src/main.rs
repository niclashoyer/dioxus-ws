use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, error, info, warn},
    prelude::*,
};
use futures::{StreamExt, channel::mpsc};
use loro::{
    ExportMode, IntoContainerId, LoroDoc, LoroMap, LoroResult, LoroText, LoroValue, Subscription,
    ValueOrContainer,
};
use server_fn::{BoxedStream, Websocket, codec::JsonEncoding};

fn main() {
    dioxus::launch(App);
}

struct UseDocument {
    pub doc: ReadOnlySignal<LoroDoc>,
    key: ReadOnlySignal<String>,
}

fn use_document(key: ReadOnlySignal<String>) -> UseDocument {
    let mut doc = use_signal(|| LoroDoc::new());

    UseDocument {
        doc: doc.into(),
        key,
    }
}

impl UseDocument {
    fn save(&self) {
        save_document(&(self.key)(), (self.doc)());
    }
}

fn save_document(key: &str, doc: LoroDoc) {
    use gloo_storage::{SessionStorage, Storage};
    let bytes = doc
        .export(ExportMode::snapshot())
        .expect("export document failed");
    SessionStorage::set(key, bytes).expect("save document failed");
}

fn load_document(key: &str) -> LoroDoc {
    use gloo_storage::{SessionStorage, Storage};
    let bytes: Result<Vec<u8>, _> = SessionStorage::get(key);
    let doc = LoroDoc::new();
    if let Ok(bytes) = bytes {
        doc.import(&bytes).expect("load document failed");
    }
    doc
}

fn load_document_bytes(key: &str) -> Option<Vec<u8>> {
    use gloo_storage::{SessionStorage, Storage};
    let bytes: Result<Vec<u8>, _> = SessionStorage::get(key);
    if let Ok(bytes) = bytes {
        Some(bytes)
    } else {
        None
    }
}

#[component]
fn Example() -> Element {
    let key = use_signal(|| "foodoc".into());
    let doc = use_document(key.into());
    rsx! {
        div {
            div {
                class: "p-4",
                p {
                    { format!("doc map string: {:?}", (doc.doc)().get_by_str_path("map/foobar").map(|x| x.as_value()).map(|x| x.unwrap().as_string()).map(|x| x.unwrap().as_str())) }
                }
            }
            div {
                class: "flex gap-4",

                button {
                    class: "p-4 rounded-lg bg-gray-200 cursor-pointer",
                    onclick: move |_e| {
                        let doc2 = LoroDoc::new();
                        doc2.import(&doc.doc.read().export(loro::ExportMode::Snapshot).unwrap()).expect("import failed");
                        doc.doc.read().import(&doc2.export(loro::ExportMode::Snapshot).unwrap()).expect("import failed");
                    },
                    "reimport"
                }
                button {
                    class: "p-4 rounded-lg bg-gray-200 cursor-pointer",
                    onclick: move |_e| {
                        doc.save();
                    },
                    "save"
                }
            }
        }
    }
}

#[component]
fn App() -> Element {
    let mut tx = use_signal(|| None);
    spawn(async move {
        let (tx_, rx) = mpsc::channel(1);
        tx.set(Some(tx_));
        match echo(rx.into()).await {
            Ok(mut msgs) => {
                while let Some(msg) = msgs.next().await {
                    debug!("received from server: {:?}", msg);
                }
            }
            Err(e) => {
                warn!("{e}")
            }
        }
    });

    rsx! {
        document::Script {
            src: "https://cdn.jsdelivr.net/npm/@tailwindcss/browser@4.1.6/dist/index.global.min.js"
        }

        div {
            class: "p-8",

            button {
                class: "p-4 bg-gray-800 text-white cursor-pointer rounded-lg",
                onclick: move |_e| async move {
                    info!("sending...");
                    tx().unwrap().try_send(Ok("button clicked!".into())).expect("send failed");
                },
                "Send something!"
            }
        }

        div {
            class: "p-8",

            Example {}
        }
    }
}

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn echo(
    input: BoxedStream<String, ServerFnError>,
) -> Result<BoxedStream<String, ServerFnError>, ServerFnError> {
    use futures::channel::mpsc;
    use futures::{SinkExt, StreamExt};
    let mut input = input;

    let (mut tx, rx) = mpsc::channel(1);

    tokio::spawn(async move {
        while let Some(i) = input.next().await {
            match i {
                Ok(s) => {
                    tx.try_send(Ok(s)).unwrap();
                }
                Err(e) => {
                    error!("{e}");
                }
            }
        }
    });

    Ok(rx.into())
}
