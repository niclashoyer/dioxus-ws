use core::fmt;
use std::{error::Error, sync::Arc};

use async_broadcast::{InactiveReceiver, Receiver, Sender, broadcast};
use dioxus::{
    logger::tracing::{debug, error, info, warn},
    prelude::{
        server_fn::{BoxedStream, Websocket, codec::JsonEncoding},
        *,
    },
};
use futures::{StreamExt, channel::mpsc};
use loro::{ContainerID, LoroDoc, ToJson};

mod storage;
use storage::{Document, DocumentId, Storage, server::RemoteMemoryStorage, web::SessionStorage};
use uuid::Uuid;

fn main() {
    dioxus::launch(App);
}

#[derive(Debug)]
struct UseStorage<S: Storage + 'static> {
    storage: Signal<S>,
}

impl<S: Storage + 'static> Clone for UseStorage<S> {
    fn clone(&self) -> Self {
        UseStorage {
            storage: self.storage.clone(),
        }
    }
}

impl<S: Storage + 'static> Copy for UseStorage<S> {}

fn use_storage<S: Storage>() -> UseStorage<S> {
    UseStorage {
        storage: use_signal(|| S::new()),
    }
}

impl<S: Storage> UseStorage<S> {
    async fn load_document(&mut self, id: &DocumentId) -> Document {
        self.storage.write().load_document(id).await
    }

    async fn save_document(&mut self, doc: &Document) {
        self.storage.write().save_document(doc).await;
    }
}

fn use_load_document<S: Storage + 'static>(
    mut storage: UseStorage<S>,
    id: ReadOnlySignal<DocumentId>,
) -> Resource<Document> {
    debug!("use_load_document");
    use_resource(move || async move {
        debug!("use_load_document/use_resource");
        let res = storage.load_document(&id()).await;
        debug!("use_load_document/use_resource: result: {:?}", res);
        res
    })
}

#[derive(Debug, Clone)]
pub enum DiffEvent {
    Load,
    Local(Option<ContainerID>),
    Import(Option<ContainerID>),
    Checkout(Option<ContainerID>),
}

#[derive(Debug, Clone, Copy)]
struct UseDocument {
    doc: Resource<Document>,
    inactive_receiver: Signal<InactiveReceiver<DiffEvent>>,
}

fn use_document<S: Storage + 'static>(
    id: ReadOnlySignal<DocumentId>,
) -> Result<UseDocument, RenderError> {
    debug!("use_document");
    let storage = use_storage::<S>();
    let doc = use_load_document(storage, id);
    let mut subscription = use_signal(|| None);
    let (tx, rx): (SyncSignal<Sender<DiffEvent>>, Signal<_>) = use_hook(|| {
        let (tx, rx) = broadcast::<DiffEvent>(1);
        (Signal::new_maybe_sync(tx), Signal::new(rx.deactivate()))
    });
    use_memo(move || {
        debug!("use_document/use_memo");
        match &*doc.value().read_unchecked() {
            Some(doc) => {
                debug!("use_document/use_memo/read_unchecked");
                let sub = doc.get_loro_doc().subscribe_root(Arc::new(move |e| {
                    debug!("use_document/use_memo/subscribe_root");
                    let e = match e.triggered_by {
                        loro::EventTriggerKind::Local => DiffEvent::Local(e.current_target),
                        loro::EventTriggerKind::Checkout => DiffEvent::Checkout(e.current_target),
                        loro::EventTriggerKind::Import => DiffEvent::Import(e.current_target),
                    };
                    spawn(async move {
                        tx.peek().broadcast(e).await.expect("send event failed");
                    });
                }));
                subscription.set(Some(sub));
                spawn(async move {
                    tx.peek()
                        .broadcast(DiffEvent::Load)
                        .await
                        .expect("send load event failed");
                });
            }
            _ => {}
        }
    });
    Ok(UseDocument {
        doc: doc.into(),
        inactive_receiver: rx,
    })
}

impl UseDocument {
    pub fn receiver(&mut self) -> Receiver<DiffEvent> {
        self.inactive_receiver.peek().clone().activate()
    }
}

fn use_document_value<T: ToLoro + FromLoro + Default + fmt::Debug + PartialEq>(
    mut doc: UseDocument,
) -> Signal<T> {
    let mut value = use_signal(|| T::default());
    use_memo(move || {
        spawn(async move {
            let mut receiver = doc.receiver();
            loop {
                let diff = receiver.recv().await;
                debug!("diff event: {:?}", diff);
                let new_value = T::from_loro(
                    &doc.doc
                        .read()
                        .as_ref()
                        .expect("document should not be None when an event is emitted")
                        .doc,
                    diff.ok(),
                )
                .unwrap(); // FIXME
                debug!("value: {:?}", new_value);
                // prevent unnecessary rerenders
                if *value.peek() != new_value {
                    value.set(new_value);
                }
            }
        });
    });
    use_effect(move || {
        // subscribe value even if the document is not ready yet
        let _val = value.read();
        if let Some(doc) = doc.doc.value().peek().as_ref() {
            // when the value changes, write it into the document
            // TODO: add compare check to prevent unnecessary writes
            value.read().to_loro(&doc.doc).ok();
            doc.doc.commit();
        };
    });
    value
}

trait ToLoro {
    fn to_loro(&self, doc: &LoroDoc) -> Result<(), Box<dyn Error>>;
}

trait FromLoro: Sized {
    fn from_loro(doc: &LoroDoc, diff: Option<DiffEvent>) -> Result<Self, Box<dyn Error>>;
}

#[derive(Debug, Default, PartialEq)]
struct Foobar(String);

impl ToLoro for Foobar {
    fn to_loro(&self, doc: &LoroDoc) -> Result<(), Box<dyn Error>> {
        doc.get_map("foobar")
            .insert("foobar", self.0.clone())
            .expect("to loro failed");
        debug!("ToLoro | doc: {}", doc.get_deep_value().to_json_value());
        Ok(())
    }
}

impl FromLoro for Foobar {
    fn from_loro(doc: &LoroDoc, _diff: Option<DiffEvent>) -> Result<Self, Box<dyn Error>> {
        debug!("FromLoro | doc: {}", doc.get_deep_value().to_json_value());
        let s = doc
            .get_map("foobar")
            .get("foobar")
            .and_then(|v| {
                v.get_deep_value()
                    .as_string()
                    .and_then(|s| Some(s.as_str().into()))
            })
            .unwrap_or_else(|| "".to_string());
        Ok(Foobar(s))
    }
}

#[component]
fn Example() -> Element {
    let mut id_str = use_signal(|| "foodoc".into());
    let id = use_memo(move || {
        Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("example.com/{}", id_str()).as_bytes(),
        )
    });
    let mut storage = use_storage::<RemoteMemoryStorage>();
    let doc = use_document::<RemoteMemoryStorage>(id.into())?;
    let mut foobar: Signal<Foobar> = use_document_value(doc);
    rsx! {
        div {
            div {
                class: "p-4",
                input {
                    class: "rounded-md bg-gray-100 p-2 min-w-24",
                    value: id_str,
                    onchange: move |e| {
                        id_str.set(e.value())
                    }
                }
            }
            div {
                class: "p-4",
                p {
                    { debug!("render: {:?}", foobar.read()); format!("string: {}", foobar.read().0) }
                }
                input {
                    class: "rounded-md bg-gray-100 p-2 min-w-24",
                    value: foobar.read().0.clone(),
                    onchange: move |e| {
                        foobar.set(Foobar(e.value()));
                    }
                }
            }
            div {
                class: "flex gap-4",

                button {
                    class: "p-4 rounded-lg bg-gray-200 cursor-pointer",
                    onclick: move |_e| {
                        doc.doc.read().as_ref().unwrap().get_loro_doc().get_map("foobar").insert("foobar", "foobar!").expect("string change failed");
                        debug!("doc: {}", doc.doc.read().as_ref().unwrap().get_loro_doc().get_deep_value().to_json_value());
                        doc.doc.read().as_ref().unwrap().get_loro_doc().commit();
                    },
                    "change string in document"
                }
                button {
                    class: "p-4 rounded-lg bg-gray-200 cursor-pointer",
                    onclick: move |_e| async move {
                        if let Some(doc) = doc.doc.read().as_ref() {
                            storage.save_document(doc).await;
                        } else {
                            warn!("save: no document");
                        }
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
    use futures::StreamExt;
    use futures::channel::mpsc;
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
