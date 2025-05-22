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
    use_memo(move || {
        #[cfg(feature = "web")]
        {
            let doc2 = LoroDoc::new();
            if let Some(bytes) = load_document_bytes(&key()) {
                doc2.import(&bytes).expect("import failed");
            }
            doc.set(doc2);
        }
    });
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

#[derive(Debug, Clone, Copy)]
struct UseDocumentText {
    pub text: SyncSignal<LoroText>,
    sub: Signal<Subscription>,
}

impl UseDocumentText {
    pub fn insert(&mut self, pos: usize, s: &str) -> LoroResult<()> {
        self.text.write().insert(pos, s)
    }

    pub fn to_string(&self) -> String {
        self.text.read().to_string()
    }
}

fn use_document_text<D: Into<ReadOnlySignal<LoroDoc>>, I: IntoContainerId>(
    doc: D,
    id: I,
) -> UseDocumentText {
    use_hook(|| {
        let doc = doc.into();
        let text = SyncSignal::new_maybe_sync(doc.read().get_text(id));
        debug!("{:?}", text);
        let sub = doc.read().subscribe(
            &text.read().id(),
            Arc::new(move |_e| {
                debug!("UseDocumentText subscriber");
                to_owned![text];
                text.write_unchecked();
            }),
        );
        let sub = Signal::new(sub);
        UseDocumentText { text, sub }
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct UseDocumentMap {
    pub map: SyncSignal<LoroMap>,
    sub: Signal<Subscription>,
}

fn use_document_map<D: Into<ReadOnlySignal<LoroDoc>>, I: IntoContainerId + Clone + 'static>(
    doc: D,
    id: I,
) -> Memo<UseDocumentMap> {
    let doc = doc.into();
    let mut map = use_hook(|| SyncSignal::new_maybe_sync(LoroMap::new()));
    use_memo(move || {
        to_owned![id];
        map.set(doc.try_read().expect("use_document_map failed").get_map(id));
        let sub = doc.read().subscribe(
            &map.read().id(),
            Arc::new(move |_e| {
                debug!("map subscriber");
                to_owned![map];
                map.write_unchecked();
            }),
        );
        let sub = Signal::new(sub);
        UseDocumentMap { map, sub }
    })
}

fn use_map_value(map: UseDocumentMap, key: &'static str) -> Memo<Option<LoroValue>> {
    use_memo(move || {
        let vc = map.map.read().get(key);
        if let Some(ValueOrContainer::Value(val)) = vc {
            Some(val)
        } else {
            None
        }
    })
}

fn use_map_string(map: UseDocumentMap, key: &'static str) -> Memo<String> {
    let val = use_map_value(map, key);
    use_memo(move || {
        if let Some(LoroValue::String(s)) = val() {
            (*s).clone()
        } else {
            "".into()
        }
    })
}

fn use_map_string_mut(map: UseDocumentMap, key: &'static str) -> Signal<String> {
    let mut sig = use_signal(|| "".into());
    let val = use_map_value(map, key);
    use_memo(move || {
        let val = val();
        let s = if let Some(LoroValue::String(s)) = val {
            (*s).clone()
        } else {
            "".into()
        };
        if *sig.peek() != s {
            sig.set(s);
        }
    });
    use_memo(move || {
        let val: String = sig();
        map.map
            .write_unchecked()
            .insert(key, LoroValue::String(val.into()))
            .expect("inserting string failed");
    });
    sig
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
    let mut text = use_document_text(doc.doc, "foo");
    let map = use_document_map(doc.doc, "bar");
    let mut string = use_map_string_mut(map(), "baz");
    rsx! {
        div {
            div {
                class: "p-4",
                p {
                    { format!("foo: {}", text.to_string()) }
                }
                p {
                    { format!("baz: {}", string) }
                }
                p {
                    { format!("baz map: {:?}", map().map.read().get("baz")) }
                }
                input {
                    class: "bg-gray-200 rounded-md p-2 mt-2",
                    type: "text",
                    oninput: move |e| string.set(e.value()),
                    value: string
                }
            }
            div {
                class: "flex gap-4",

                button {
                    class: "p-4 rounded-lg bg-gray-200 cursor-pointer",
                    onclick: move |_e| {
                        text.insert(0, "foo").expect("foo failed");
                    },
                    "add foo"
                }
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
                        map().map.write_unchecked().insert("baz", "buz").expect("baz failed");
                    },
                    "baz"
                }
                button {
                    class: "p-4 rounded-lg bg-gray-200 cursor-pointer",
                    onclick: move |_e| {
                        string.set("boz".into());
                    },
                    "boz"
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
