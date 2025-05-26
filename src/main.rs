use core::fmt;
use std::{any::Any, error::Error, sync::Arc};

use dioxus::{
    logger::tracing::{debug, error, info, warn},
    prelude::*,
};
use futures::{SinkExt, StreamExt, channel::mpsc};
use loro::{
    ContainerID, ContainerTrait, ExportMode, IntoContainerId, LoroDoc, LoroMap, LoroResult,
    LoroText, LoroValue, Subscription, ToJson, ValueOrContainer, event::Subscriber,
};
// use server_fn::{BoxedStream, Websocket, codec::JsonEncoding};
use async_broadcast::{InactiveReceiver, Receiver, SendError, Sender, TrySendError, broadcast};

mod storage;
use storage::{Document, DocumentId, Storage};

fn main() {
    dioxus::launch(App);
}

fn use_load_document(id: ReadOnlySignal<DocumentId>) -> Result<Resource<Document>, RenderError> {
    debug!("use_document");
    let resource = use_resource(move || async move {
        debug!("use_document/use_resource");
        let mut storage = storage::web::SessionStorage::default(); // FIXME: refactor this into a context or static
        storage.load_document(&id()).await
    });

    // This is a simple hack to check if the resource is ready on the first run. Taken from:
    // https://docs.rs/dioxus-fullstack/0.6.3/src/dioxus_fullstack/hooks/server_future.rs.html#60-65
    // On the first run, force this task to be polled right away in case its value is ready
    use_hook(|| {
        let _ = resource.task().poll_now();
    });

    // Suspend if the value isn't ready
    if resource.state().cloned() == UseResourceState::Pending {
        let task = resource.task();

        if !task.paused() {
            return Err(suspend(task).unwrap_err());
        }
    }

    Ok(resource)
}

#[derive(Debug, Clone)]
enum DiffEvent {
    Load,
    Local(Option<ContainerID>),
    Import(Option<ContainerID>),
    Checkout(Option<ContainerID>),
}

#[derive(Debug, Clone, Copy)]
struct UseDocument {
    doc: ReadOnlySignal<Option<Document>>,
    inactive_receiver: Signal<InactiveReceiver<DiffEvent>>,
}

fn use_document(doc: ReadOnlySignal<Option<Document>>) -> UseDocument {
    let mut subscription = use_signal(|| None);
    let (tx, rx): (SyncSignal<Sender<DiffEvent>>, Signal<_>) = use_hook(|| {
        let (tx, rx) = broadcast::<DiffEvent>(1);
        (Signal::new_maybe_sync(tx), Signal::new(rx.deactivate()))
    });
    use_memo(move || {
        match &*doc.read_unchecked() {
            Some(doc) => {
                let sub = doc.get_loro_doc().subscribe_root(Arc::new(move |e| {
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
            _ => {
                // TODO: check for error, see https://docs.rs/dioxus/latest/dioxus/prelude/struct.Resource.html#method.value
            }
        }
    });
    UseDocument {
        doc,
        inactive_receiver: rx,
    }
}

impl UseDocument {
    pub fn receiver(&mut self) -> Receiver<DiffEvent> {
        self.inactive_receiver.peek().clone().activate()
    }
}

fn use_document_value<T: ToLoro + FromLoro + Default + fmt::Debug>(
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
                value.set(new_value);
            }
        });
    });
    use_effect(move || {
        doc.doc.peek().as_ref().and_then(move |doc| {
            value.read().to_loro(&doc.doc).ok();
            doc.doc.commit();
            Some(())
        });
    });
    value
}

trait ToLoro {
    fn to_loro(&self, doc: &LoroDoc) -> Result<(), Box<dyn Error>>;
}

trait FromLoro: Sized {
    fn from_loro(doc: &LoroDoc, diff: Option<DiffEvent>) -> Result<Self, Box<dyn Error>>;
}

#[derive(Debug, Default)]
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
    let mut id = use_signal(|| "foodoc".into());
    let doc_resource = use_load_document(id.into())?;
    let doc = use_document(doc_resource.value());
    let mut foobar: Signal<Foobar> = use_document_value(doc);
    rsx! {
        div {
            div {
                class: "p-4",
                p {
                    // { debug!("render: {}", string.s.read()); format!("string: {}", string.s.read()) }
                }
                input {
                    class: "rounded-md bg-gray-100 p-2 min-w-24",
                    value: id,
                    onchange: move |e| {
                        id.set(e.value())
                    }
                }
                // input {
                //     class: "rounded-md bg-gray-100 p-2 min-w-24",
                //     value: string.s,
                //     onchange: move |e| {
                //         string.s.set(e.value())
                //     }
                // }
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

                // button {
                //     class: "p-4 rounded-lg bg-gray-200 cursor-pointer",
                //     onclick: move |_e| {
                //         let doc2 = LoroDoc::new();
                //         doc2.import(&doc.doc.read().export(loro::ExportMode::Snapshot).unwrap()).expect("import failed");
                //         doc.doc.read().import(&doc2.export(loro::ExportMode::Snapshot).unwrap()).expect("import failed");
                //     },
                //     "reimport"
                // }
                button {
                    class: "p-4 rounded-lg bg-gray-200 cursor-pointer",
                    onclick: move |_e| {
                        doc.doc.read().as_ref().unwrap().get_loro_doc().get_map("foomap").insert("foobar", "foobar!").expect("string change failed");
                        debug!("doc: {}", doc.doc.read().as_ref().unwrap().get_loro_doc().get_deep_value().to_json_value());
                    },
                    "change string in document"
                }
                button {
                    class: "p-4 rounded-lg bg-gray-200 cursor-pointer",
                    onclick: move |_e| async move {
                        let mut storage = storage::web::SessionStorage::default(); // FIXME: refactor this into a context or static
                        storage.save_document(doc.doc.read().as_ref().unwrap()).await;
                    },
                    "save"
                }
            }
        }
    }
}

#[component]
fn App() -> Element {
    // let mut tx = use_signal(|| None);
    // spawn(async move {
    //     let (tx_, rx) = mpsc::channel(1);
    //     tx.set(Some(tx_));
    //     match echo(rx.into()).await {
    //         Ok(mut msgs) => {
    //             while let Some(msg) = msgs.next().await {
    //                 debug!("received from server: {:?}", msg);
    //             }
    //         }
    //         Err(e) => {
    //             warn!("{e}")
    //         }
    //     }
    // });

    rsx! {
        document::Script {
            src: "https://cdn.jsdelivr.net/npm/@tailwindcss/browser@4.1.6/dist/index.global.min.js"
        }

        div {
            class: "p-8",

            // button {
            //     class: "p-4 bg-gray-800 text-white cursor-pointer rounded-lg",
            //     onclick: move |_e| async move {
            //         info!("sending...");
            //         tx().unwrap().try_send(Ok("button clicked!".into())).expect("send failed");
            //     },
            //     "Send something!"
            // }
        }

        div {
            class: "p-8",

            Example {}
        }
    }
}

// #[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
// pub async fn echo(
//     input: BoxedStream<String, ServerFnError>,
// ) -> Result<BoxedStream<String, ServerFnError>, ServerFnError> {
//     use futures::channel::mpsc;
//     use futures::{SinkExt, StreamExt};
//     let mut input = input;

//     let (mut tx, rx) = mpsc::channel(1);

//     tokio::spawn(async move {
//         while let Some(i) = input.next().await {
//             match i {
//                 Ok(s) => {
//                     tx.try_send(Ok(s)).unwrap();
//                 }
//                 Err(e) => {
//                     error!("{e}");
//                 }
//             }
//         }
//     });

//     Ok(rx.into())
// }
