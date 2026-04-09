use crate::{
    app::{
        ActiveTab, Language, TransferEvent, component_banner_sharedfile::SharedFileBanner,
        send_path,
    },
    crypto::RsaPublicKey,
    ipc::{DiscoveredDevice, OutgoingTransfer, SessionPassword, TransferStatus},
    pick_file::{pick_file_to_send, pick_folder_to_send},
};
use anyhow::{self as ah, format_err as err};
use dioxus::prelude::*;
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::mpsc;

fn next_id(mut next_send_id: Signal<u64>) -> u64 {
    let mut id = next_send_id.write();
    let current = *id;
    *id += 1;
    current
}

fn path_filename(path: &Path) -> ah::Result<String> {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| err!("Invalid file name"))
}

#[allow(clippy::too_many_arguments)]
fn send_it(
    etx: Option<mpsc::UnboundedSender<TransferEvent>>,
    addr: SocketAddr,
    path: PathBuf,
    sender: String,
    peer_rsa_pub_key: RsaPublicKey,
    session_password: SessionPassword,
    tid: u64,
    mut active_tab: Signal<ActiveTab>,
) {
    active_tab.set(ActiveTab::Outgoing);
    if let Some(etx) = etx {
        tokio::spawn(async move {
            if let Err(e) = send_path(
                addr,
                path,
                sender,
                peer_rsa_pub_key,
                session_password,
                tid,
                etx,
            )
            .await
            {
                log::warn!("Failed to send: {e}");
            }
        });
    }
}

#[component]
pub fn NetworkPanel(
    devices: Signal<HashMap<[u8; 32], DiscoveredDevice>>,
    event_tx: Signal<Option<mpsc::UnboundedSender<TransferEvent>>>,
    device_name: Signal<String>,
    next_send_id: Signal<u64>,
    outgoing_transfers: Signal<Vec<OutgoingTransfer>>,
    shared_files: Signal<Vec<PathBuf>>,
    active_tab: Signal<ActiveTab>,
) -> Element {
    let session_password: SessionPassword = use_context();
    let lang = use_context::<Signal<Language>>();
    let l = *lang.read();
    let has_shared = !shared_files.read().is_empty();
    let devs = devices.read();
    if devs.is_empty() {
        return rsx! {
            if has_shared {
                SharedFileBanner { shared_files }
            }
            div { class: "empty",
                p { {l.no_devices()} }
                p { class: "hint", {l.no_devices_hint()} }
            }
        };
    }

    let dev_list: Vec<([u8; 32], String, SocketAddr, u16, RsaPublicKey)> = devs
        .values()
        .map(|d| {
            (
                d.fingerprint,
                d.device_name.clone(),
                d.addr,
                d.transfer_port,
                d.rsa_public_key.clone(),
            )
        })
        .collect();

    rsx! {
        if has_shared {
            SharedFileBanner { shared_files }
        }
        div { class: "device-list",
            for (fp, name, addr, _port, rsa_key) in dev_list {
                div {
                    class: "device-card",
                    key: "{crate::crypto::fingerprint_hex(&fp)}",
                    div { class: "device-info",
                        span { class: "device-name", "{name}" }
                        span { class: "device-addr", "{addr.ip()}" }
                    }
                    div { class: "device-actions",
                        if has_shared {
                            button {
                                class: "send-btn",
                                onclick: {
                                    let target_name = name.clone();
                                    let sp = Arc::clone(&session_password);
                                    let rsa_key = rsa_key.clone();
                                    move |_| {
                                        let target_name = target_name.clone();
                                        let etx = event_tx.read().clone();
                                        let sender = device_name.read().clone();
                                        let sp = Arc::clone(&sp);
                                        let rsa_key = rsa_key.clone();
                                        let pending_shared: Vec<PathBuf> =
                                            shared_files.read().clone();
                                        shared_files.write().clear();
                                        for path in pending_shared {
                                            let target_name = target_name.clone();
                                            let etx = etx.clone();
                                            let sender = sender.clone();
                                            let sp = Arc::clone(&sp);
                                            let rsa_key = rsa_key.clone();
                                            let tid = next_id(next_send_id);
                                            spawn(async move {
                                                if let Ok(filename) = path_filename(&path) {
                                                    outgoing_transfers
                                                        .write()
                                                        .push(OutgoingTransfer {
                                                            id: tid,
                                                            filename,
                                                            file_size: 0,
                                                            target_device: target_name,
                                                            status: TransferStatus::Pending,
                                                        });
                                                    send_it(etx, addr, path, sender, rsa_key, sp, tid, active_tab);
                                                }
                                            });
                                        }
                                    }
                                },
                                {l.send_shared()}
                            }
                        } else {
                            button {
                                class: "send-btn",
                                onclick: {
                                    let target_name = name.clone();
                                    let sp = Arc::clone(&session_password);
                                    let rsa_key = rsa_key.clone();
                                    move |_| {
                                        let target_name = target_name.clone();
                                        let etx = event_tx.read().clone();
                                        let sender = device_name.read().clone();
                                        let sp = Arc::clone(&sp);
                                        let rsa_key = rsa_key.clone();
                                        let tid = next_id(next_send_id);
                                        outgoing_transfers
                                            .write()
                                            .push(OutgoingTransfer {
                                                id: tid,
                                                filename: String::new(),
                                                file_size: 0,
                                                target_device: target_name.clone(),
                                                status: TransferStatus::Pending,
                                            });
                                        active_tab.set(ActiveTab::Outgoing);
                                        spawn(async move {
                                            let file = pick_file_to_send(*lang.read()).await;
                                            if let Some(path) = file {
                                                if let Ok(filename) = path_filename(&path) {
                                                    {
                                                        let mut list = outgoing_transfers.write();
                                                        if let Some(t) = list.iter_mut().find(|t| t.id == tid) {
                                                            t.filename = filename;
                                                        }
                                                    }
                                                    send_it(etx, addr, path, sender, rsa_key, sp, tid, active_tab);
                                                }
                                            } else {
                                                outgoing_transfers.write().retain(|t| t.id != tid);
                                            }
                                        });
                                    }
                                },
                                {l.send_file()}
                            }
                            button {
                                class: "send-btn",
                                onclick: {
                                    let target_name = name.clone();
                                    let sp = Arc::clone(&session_password);
                                    let rsa_key = rsa_key.clone();
                                    move |_| {
                                        let target_name = target_name.clone();
                                        let etx = event_tx.read().clone();
                                        let sender = device_name.read().clone();
                                        let sp = Arc::clone(&sp);
                                        let rsa_key = rsa_key.clone();
                                        let tid = next_id(next_send_id);
                                        outgoing_transfers
                                            .write()
                                            .push(OutgoingTransfer {
                                                id: tid,
                                                filename: String::new(),
                                                file_size: 0,
                                                target_device: target_name.clone(),
                                                status: TransferStatus::Pending,
                                            });
                                        active_tab.set(ActiveTab::Outgoing);
                                        spawn(async move {
                                            let folder = pick_folder_to_send(*lang.read()).await;
                                            if let Some(path) = folder {
                                                if let Ok(filename) = path_filename(&path) {
                                                    {
                                                        let mut list = outgoing_transfers.write();
                                                        if let Some(t) = list.iter_mut().find(|t| t.id == tid) {
                                                            t.filename = filename;
                                                        }
                                                    }
                                                    send_it(etx, addr, path, sender, rsa_key, sp, tid, active_tab);
                                                }
                                            } else {
                                                outgoing_transfers.write().retain(|t| t.id != tid);
                                            }
                                        });
                                    }
                                },
                                {l.send_folder()}
                            }
                        }
                    }
                }
            }
        }
    }
}
