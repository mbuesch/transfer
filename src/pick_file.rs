use crate::l10n::Language;
use std::path::PathBuf;

pub async fn pick_file_to_send(lang: Language) -> Option<PathBuf> {
    let ret;

    #[cfg(target_os = "android")]
    {
        ret = crate::android_interface::android_pick_file_to_send(lang).await;
    }

    #[cfg(not(target_os = "android"))]
    {
        let fh = rfd::AsyncFileDialog::new()
            .set_title(lang.select_file_title())
            .pick_file()
            .await;
        ret = fh.map(|h| PathBuf::from(h.path()));
    }

    ret
}

pub async fn pick_folder_to_send(lang: Language) -> Option<PathBuf> {
    let ret;

    #[cfg(target_os = "android")]
    {
        ret = crate::android_interface::android_pick_folder_to_send(lang).await;
    }

    #[cfg(not(target_os = "android"))]
    {
        let fh = rfd::AsyncFileDialog::new()
            .set_title(lang.select_folder_to_send_title())
            .pick_folder()
            .await;
        ret = fh.map(|h| PathBuf::from(h.path()));
    }

    ret
}

pub async fn pick_save_folder(lang: Language) -> Option<PathBuf> {
    let ret;

    #[cfg(target_os = "android")]
    {
        ret = crate::android_interface::android_pick_save_folder(lang).await;
    }

    #[cfg(not(target_os = "android"))]
    {
        let fh = rfd::AsyncFileDialog::new()
            .set_title(lang.select_folder_title())
            .pick_folder()
            .await;
        ret = fh.map(|h| PathBuf::from(h.path()));
    }

    ret
}
