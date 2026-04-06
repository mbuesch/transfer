use crate::{
    app::{Language, render_status},
    ipc::{IncomingTransfer, TransferStatus},
    pick_file::pick_save_folder,
    protocol::transfer::TransferCommand,
};
use bytesize::ByteSize;
use dioxus::prelude::*;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[component]
pub fn IncomingPanel(
    transfers: Signal<Vec<IncomingTransfer>>,
    cmd_tx: Signal<Option<mpsc::UnboundedSender<TransferCommand>>>,
    auto_accept_folder: Signal<Option<PathBuf>>,
) -> Element {
    let lang = use_context::<Signal<Language>>();
    let l = *lang.read();
    let list = transfers.read();

    rsx! {
        div { class: "auto-accept-box",
            span { class: "auto-accept-label", {l.auto_accept_folder_label()} }
            div { class: "auto-accept-row",
                {
                    let folder_display = auto_accept_folder
                        .read()
                        .as_ref()
                        .map(|p| p.display().to_string());
                    if let Some(path_str) = folder_display {
                        rsx! {
                            span { class: "auto-accept-path", "{path_str}" }
                            button {
                                class: "auto-accept-clear-btn",
                                onclick: move |_| {
                                    auto_accept_folder.set(None);
                                },
                                {l.clear_auto_accept_folder()}
                            }
                        }
                    } else {
                        rsx! {
                            span { class: "auto-accept-none", {l.auto_accept_folder_none()} }
                        }
                    }
                }
                button {
                    class: "auto-accept-select-btn",
                    onclick: move |_| {
                        spawn(async move {
                            let folder = pick_save_folder(*lang.read()).await;
                            if let Some(path) = folder {
                                auto_accept_folder.set(Some(path));
                            }
                        });
                    },
                    {l.select_auto_accept_folder()}
                }
            }
        }
        if list.is_empty() {
            div { class: "empty",
                p { {l.no_incoming()} }
            }
        } else {
            div { class: "transfer-list",
                for t in list.iter().rev() {
                    div { class: "transfer-card", key: "{t.id}",
                        div { class: "transfer-info",
                            span { class: "transfer-filename", "{t.header.filename}" }
                            span { class: "transfer-size", "{ByteSize(t.header.file_size).to_string()}" }
                            span { class: "transfer-from",
                                {l.from_label(&t.header.sender_name.as_str_lossy())}
                            }
                        }
                        div { class: "transfer-right",
                            {
                                match &t.status {
                                    TransferStatus::Completed => {
                                        if let Some(path) = &t.save_path {
                                            rsx! {
                                                div {
                                                    p { class: "transfer-path", "{path.display()}" }
                                                    div { class: "transfer-status", {render_status(&t.status, l)} }
                                                }
                                            }
                                        } else {
                                            rsx! {
                                                div { class: "transfer-status", {render_status(&t.status, l)} }
                                            }
                                        }
                                    }
                                    _ => rsx! {
                                        div { class: "transfer-status", {render_status(&t.status, l)} }
                                    },
                                }
                            }
                            {
                                match &t.status {
                                    TransferStatus::Pending => {
                                        let tid = t.id;
                                        rsx! {
                                            div { class: "transfer-actions",
                                                button {
                                                    class: "accept-btn",
                                                    onclick: move |_| {
                                                        spawn(async move {
                                                            let folder = pick_save_folder(*lang.read()).await;
                                                            if let Some(save_path) = folder && let Some(tx) = cmd_tx.read().as_ref() {
                                                                let _ = tx
                                                                    .send(TransferCommand::AcceptTransfer {
                                                                        transfer_id: tid,
                                                                        save_path,
                                                                    });
                                                            }
                                                        });
                                                    },
                                                    {l.accept()}
                                                }
                                                button {
                                                    class: "reject-btn",
                                                    onclick: move |_| {
                                                        if let Some(tx) = cmd_tx.read().as_ref() {
                                                            let _ = tx
                                                                .send(TransferCommand::RejectTransfer {
                                                                    transfer_id: tid,
                                                                });
                                                        }
                                                    },
                                                    {l.reject()}
                                                }
                                            }
                                        }
                                    }
                                    _ => rsx! {},
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
