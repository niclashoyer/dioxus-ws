use dioxus::{
    logger::tracing::{debug, error, info, warn},
    prelude::*,
};
use futures::{SinkExt, StreamExt, channel::mpsc};
use server_fn::{BoxedStream, Websocket, codec::JsonEncoding};

fn main() {
    dioxus::launch(App);
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
